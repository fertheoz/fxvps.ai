//! In-process trading stack: fix-gateway -> core-engine (with
//! [`FixLpRouter`]) -> [`InProcessCore`]. Used by client-gateway `--demo`
//! and by tests; production splits these into processes (NATS follow-up).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fix_gateway::{GatewayConfig, GatewayHandle};
use money::{Currency, Money, SCALE};
use oms::{Command, Event};
use risk::{GroupConfig, MarginMode, Routing, SymbolSpec};
use tokio::sync::broadcast;

use crate::api::{AccountNames, CoreError, InProcessCore};
use crate::lp_fix::{run_bridge, FixLpRouter, SymbolMap};
use crate::output::Publisher;
use crate::{spawn_with, EngineHandle, LpFeedback, LpMode, Settings};

/// Accounts, group and symbols created on a fresh journal.
#[derive(Clone, Debug)]
pub struct Seed {
    /// (external account id, engine account number)
    pub accounts: Vec<(String, u64)>,
    pub deposit: Money,
    pub group: GroupConfig,
    pub symbols: Vec<SymbolSpec>,
}

impl Seed {
    /// `n` demo accounts `DEMO-1..n` with `deposit` each in an ESMA retail
    /// group (1:30 on major FX, netting, A-book, small markup) and FX
    /// symbols derived from the gateway instruments.
    pub fn demo(gw: &GatewayConfig, n: u64, deposit: Money) -> Seed {
        let mut group = GroupConfig::retail("demo-retail", deposit.currency, Routing::ABook);
        group.margin_mode = MarginMode::Netting;
        group.markup_points = 5;
        Seed {
            accounts: (1..=n).map(|i| (format!("DEMO-{i}"), i)).collect(),
            deposit,
            group,
            symbols: gw
                .instruments
                .iter()
                .filter_map(|i| fx_spec(&i.symbol, i.tick_size))
                .collect(),
        }
    }

    fn commands(&self) -> Vec<Command> {
        let mut v: Vec<Command> = self
            .symbols
            .iter()
            .cloned()
            .map(Command::AddSymbol)
            .collect();
        v.push(Command::SetGroup(self.group.clone()));
        for (_, no) in &self.accounts {
            v.push(Command::OpenAccount {
                account: *no,
                group: self.group.name.clone(),
            });
            v.push(Command::Deposit {
                account: *no,
                amount: self.deposit,
                key: "seed".into(),
            });
        }
        v
    }
}

/// Standard FX spec for an LP symbol like `EUR/USD` (commission USD 3.50 per
/// lot per side).
pub fn fx_spec(lp_symbol: &str, tick: domain::Price) -> Option<SymbolSpec> {
    let (b, q) = lp_symbol.split_once('/')?;
    let base: Currency = b.parse().ok()?;
    let quote: Currency = q.parse().ok()?;
    let mut digits = 0;
    let mut t = tick.raw();
    while t > 0 && t < SCALE && digits < 8 {
        t *= 10;
        digits += 1;
    }
    let mut s = SymbolSpec::fx(&core_symbol(lp_symbol), base, quote, digits);
    s.commission_per_lot = Money::parse("3.50", Currency::USD).ok()?;
    Some(s)
}

/// `EUR/USD` -> `EURUSD` (same as the client wire symbol).
pub fn core_symbol(lp_symbol: &str) -> String {
    lp_symbol.replace('/', "")
}

#[derive(Clone, Debug)]
pub struct StackConfig {
    pub gateway: GatewayConfig,
    pub data_dir: PathBuf,
    pub snapshot_every: u64,
    /// Applied when the journal holds no accounts yet.
    pub seed: Option<Seed>,
    /// cl_ord_id prefix of omnibus LP orders.
    pub lp_prefix: String,
    /// Minimum interval between quote-driven P&L pushes per account.
    pub pnl_push_interval: Duration,
}

impl StackConfig {
    pub fn new(gateway: GatewayConfig, data_dir: impl Into<PathBuf>) -> StackConfig {
        StackConfig {
            gateway,
            data_dir: data_dir.into(),
            snapshot_every: 1_000,
            seed: None,
            lp_prefix: "LP-".into(),
            pnl_push_interval: Duration::from_millis(250),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StackError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("gateway: {0}")]
    Gateway(#[from] fix_gateway::GatewayError),
    #[error("core: {0}")]
    Core(#[from] CoreError),
    #[error("engine: {0}")]
    Engine(String),
}

pub struct CoreStack {
    pub core: Arc<InProcessCore>,
    pub engine: EngineHandle,
    gateway: GatewayHandle,
    writer: std::thread::JoinHandle<()>,
    bridge: tokio::task::JoinHandle<()>,
}

impl CoreStack {
    pub async fn start(cfg: StackConfig) -> Result<CoreStack, StackError> {
        let mut symbols = SymbolMap::default();
        for i in &cfg.gateway.instruments {
            symbols.insert(&core_symbol(&i.symbol), &i.symbol, 100_000);
        }
        let symbols = Arc::new(symbols);
        let gateway = fix_gateway::start(cfg.gateway.clone())?;
        // subscribe before anything else so the first MD snapshot is not missed
        let gw_events = gateway.subscribe();
        let feedback = LpFeedback::default();
        let router = FixLpRouter::new(
            gateway.orders(),
            feedback.clone(),
            symbols.clone(),
            cfg.lp_prefix.clone(),
        );
        let names = AccountNames::default();
        if let Some(seed) = &cfg.seed {
            for (n, no) in &seed.accounts {
                names.insert(n, *no);
            }
        }
        let (events, _) = broadcast::channel(16_384);
        let publisher = Publisher::new(
            events.clone(),
            names.clone(),
            cfg.pnl_push_interval.as_nanos() as u64,
        );
        let mut settings = Settings::new(&cfg.data_dir);
        settings.snapshot_every = cfg.snapshot_every;
        settings.simulate_lp = false;
        let (engine, writer) = spawn_with(
            settings,
            LpMode::External {
                router: Box::new(router),
                feedback,
            },
            Some(publisher),
        )?;
        if let Some(seed) = &cfg.seed {
            let fresh = engine
                .read(|e| e.accounts().next().is_none())
                .await
                .map_err(StackError::Engine)?;
            if fresh {
                for c in seed.commands() {
                    let ev = engine.command(c).await.map_err(StackError::Engine)?;
                    if let Some(Event::CommandRejected { reason }) = ev.first() {
                        tracing::warn!(%reason, "seed command rejected");
                    }
                }
            }
        }
        // Contract sizes come from the engine's symbol specs.
        let specs: Vec<(String, i64)> = engine
            .read(|e| {
                e.symbols()
                    .map(|s| (s.symbol.clone(), s.contract_size))
                    .collect()
            })
            .await
            .map_err(StackError::Engine)?;
        if specs.iter().any(|(_, cs)| *cs != 100_000) {
            tracing::warn!("non-standard contract sizes: LP mapping assumes 100000");
        }
        let bridge = tokio::spawn(run_bridge(
            engine.clone(),
            gw_events,
            symbols,
            cfg.lp_prefix.clone(),
        ));
        let core = Arc::new(InProcessCore::new(engine.clone(), events, names).await?);
        Ok(CoreStack {
            core,
            engine,
            gateway,
            writer,
            bridge,
        })
    }

    /// Stops the bridge, the FIX sessions and the engine (final snapshot).
    pub async fn shutdown(self) {
        self.bridge.abort();
        let _ = self.bridge.await;
        self.gateway.shutdown().await;
        self.engine.shutdown();
        let _ = tokio::task::spawn_blocking(move || self.writer.join()).await;
    }
}
