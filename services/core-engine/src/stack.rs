//! In-process trading stack: one fix-gateway per LP -> [`Aggregator`] ->
//! core-engine (with [`AggLpRouter`]) -> [`InProcessCore`]. Used by client-gateway `--demo`
//! and by tests; production splits these into processes (NATS follow-up).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fix_gateway::{GatewayConfig, GatewayHandle};
use money::{Currency, Money, SCALE};
use oms::{Command, Event};
use risk::{AssetClass, GroupConfig, MarginMode, Routing, SymbolSpec};
use tokio::sync::broadcast;

use crate::api::{AccountNames, CoreError, InProcessCore};
use crate::lp_agg::{AggConfig, Aggregator};
use crate::lp_fix::{run_bridge, AggLpRouter, LpLink, SymbolMap};
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
    /// Additional groups with their accounts (same deposit), e.g. a
    /// hedging variant of `group`.
    pub extra: Vec<(GroupConfig, Vec<(String, u64)>)>,
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
            extra: Vec::new(),
        }
    }

    /// Adds `n` hedging accounts `DEMO-H1..n` (engine numbers 101..) in a
    /// hedging copy of the demo group (`demo-hedge`).
    pub fn with_hedging(mut self, n: u64) -> Seed {
        let mut g = self.group.clone();
        g.name = "demo-hedge".into();
        g.margin_mode = MarginMode::Hedging;
        let accounts = (1..=n).map(|i| (format!("DEMO-H{i}"), 100 + i)).collect();
        self.extra.push((g, accounts));
        self
    }

    /// Every seeded (external id, engine number).
    pub fn all_accounts(&self) -> impl Iterator<Item = &(String, u64)> {
        self.accounts
            .iter()
            .chain(self.extra.iter().flat_map(|(_, a)| a.iter()))
    }

    fn commands(&self) -> Vec<Command> {
        let mut v: Vec<Command> = self
            .symbols
            .iter()
            .cloned()
            .map(Command::AddSymbol)
            .collect();
        v.push(Command::SetGroup(self.group.clone()));
        for (g, _) in &self.extra {
            v.push(Command::SetGroup(g.clone()));
        }
        let groups = std::iter::once((&self.group, &self.accounts))
            .chain(self.extra.iter().map(|(g, a)| (g, a)));
        for (g, no) in groups.flat_map(|(g, a)| a.iter().map(move |(_, no)| (g, no))) {
            v.push(Command::OpenAccount {
                account: *no,
                group: g.name.clone(),
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
    s.contract_size = lot_units(lp_symbol);
    s.asset_class = asset_class(b, q);
    Some(s)
}

/// Units of the base per engine lot: 100 000 for FX; spot metals in troy
/// ounces (gold, platinum, palladium 100; silver 5 000).
pub fn lot_units(lp_symbol: &str) -> i64 {
    match lp_symbol.split('/').next().unwrap_or("") {
        "XAU" | "XPT" | "XPD" => 100,
        "XAG" => 5_000,
        _ => 100_000,
    }
}

/// ESMA classes: majors = pairs of USD, EUR, JPY, GBP, CAD, CHF.
fn asset_class(base: &str, quote: &str) -> AssetClass {
    const MAJOR: [&str; 6] = ["USD", "EUR", "JPY", "GBP", "CAD", "CHF"];
    match base {
        "XAU" => AssetClass::Gold,
        "XAG" | "XPT" | "XPD" => AssetClass::Commodity,
        _ if MAJOR.contains(&base) && MAJOR.contains(&quote) => AssetClass::MajorFx,
        _ => AssetClass::MinorFx,
    }
}

/// `EUR/USD` -> `EURUSD` (same as the client wire symbol).
pub fn core_symbol(lp_symbol: &str) -> String {
    lp_symbol.replace('/', "")
}

#[derive(Clone, Debug)]
pub struct StackConfig {
    /// Primary LP (its instruments seed the engine symbols).
    pub gateway: GatewayConfig,
    /// Further LPs aggregated with the primary (stage 6); distinct `lp` names.
    pub extra_gateways: Vec<GatewayConfig>,
    /// Initial aggregation policy (the admin API changes it at runtime).
    pub aggregation: AggConfig,
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
            extra_gateways: Vec::new(),
            aggregation: AggConfig::default(),
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
    #[error("config: {0}")]
    Config(String),
}

pub struct CoreStack {
    pub core: Arc<InProcessCore>,
    pub engine: EngineHandle,
    /// Shared multi-LP book and policy (admin API `/v1/lp/aggregation`).
    pub agg: Arc<Aggregator>,
    gateways: Vec<GatewayHandle>,
    writer: std::thread::JoinHandle<()>,
    bridges: Vec<tokio::task::JoinHandle<()>>,
    status: std::sync::Arc<std::sync::RwLock<Vec<fix_gateway::SessionStatus>>>,
    status_task: Option<tokio::task::JoinHandle<()>>,
}

impl CoreStack {
    pub async fn start(cfg: StackConfig) -> Result<CoreStack, StackError> {
        let all: Vec<&GatewayConfig> = std::iter::once(&cfg.gateway)
            .chain(cfg.extra_gateways.iter())
            .collect();
        let mut lp_names = std::collections::BTreeSet::new();
        for g in &all {
            if !lp_names.insert(g.lp.as_str()) {
                return Err(StackError::Config(format!("duplicate LP name {}", g.lp)));
            }
        }
        let agg = Arc::new(Aggregator::new(cfg.aggregation.clone()));
        let feedback = LpFeedback::default();
        let mut gateways = Vec::new();
        let mut links = Vec::new();
        let mut subs = Vec::new();
        for g in &all {
            let mut symbols = SymbolMap::default();
            for i in &g.instruments {
                // LP OrderQty per engine lot (100 000 units): LMAX FX contracts are
                // 10 000 units, so 1 lot = 10 contracts; contract_size 1 = units.
                let (lot, cs) = (lot_units(&i.symbol), i.contract_size.max(1));
                if lot % cs != 0 {
                    return Err(StackError::Config(format!(
                        "{} {}: contract_size {cs} does not divide a {lot}-unit lot",
                        g.lp, i.symbol
                    )));
                }
                symbols.insert(&core_symbol(&i.symbol), &i.symbol, lot / cs);
            }
            let symbols = Arc::new(symbols);
            let gateway = fix_gateway::start((*g).clone())?;
            // subscribe before anything else so the first MD snapshot is not missed
            subs.push((g.lp.clone(), gateway.subscribe(), symbols.clone()));
            links.push(LpLink {
                name: g.lp.clone(),
                orders: gateway.orders(),
                symbols,
            });
            gateways.push(gateway);
        }
        let router = AggLpRouter::new(links, agg.clone(), feedback.clone(), cfg.lp_prefix.clone());
        let names = AccountNames::default();
        if let Some(seed) = &cfg.seed {
            for (n, no) in seed.all_accounts() {
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
        // Instruments of every LP (added after the first start too) become
        // engine symbols on the next start (the seed only runs once).
        let mut seen = std::collections::BTreeSet::new();
        for i in all.iter().flat_map(|g| g.instruments.iter()) {
            let Some(spec) = fx_spec(&i.symbol, i.tick_size) else {
                continue;
            };
            if !seen.insert(spec.symbol.clone()) {
                continue;
            }
            agg.set_point(&spec.symbol, spec.point());
            let name = spec.symbol.clone();
            let known = engine
                .read(move |e| e.symbols().any(|x| x.symbol == name))
                .await
                .map_err(StackError::Engine)?;
            let seeded = cfg
                .seed
                .as_ref()
                .is_some_and(|sd| sd.symbols.iter().any(|x| x.symbol == spec.symbol));
            if !known && !seeded {
                engine
                    .command(Command::AddSymbol(spec))
                    .await
                    .map_err(StackError::Engine)?;
            }
        }
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
        let bridges = subs
            .into_iter()
            .map(|(lp, rx, symbols)| {
                tokio::spawn(run_bridge(
                    engine.clone(),
                    rx,
                    symbols,
                    cfg.lp_prefix.clone(),
                    events.clone(),
                    lp,
                    agg.clone(),
                ))
            })
            .collect();
        // One session table over all gateways (admin API `/v1/lp/sessions`):
        // the single gateway's own table, or a merged copy refreshed twice a second.
        let (status, status_task) = if gateways.len() == 1 {
            (gateways[0].status_source(), None)
        } else {
            let sources: Vec<_> = gateways.iter().map(|g| g.status_source()).collect();
            let merged = Arc::new(std::sync::RwLock::new(Vec::new()));
            let w = merged.clone();
            let task = tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_millis(500));
                loop {
                    tick.tick().await;
                    let rows: Vec<fix_gateway::SessionStatus> = sources
                        .iter()
                        .flat_map(|s| s.read().map(|t| t.clone()).unwrap_or_default())
                        .collect();
                    if let Ok(mut t) = w.write() {
                        *t = rows;
                    }
                }
            });
            (merged, Some(task))
        };
        let core = Arc::new(InProcessCore::new(engine.clone(), events, names).await?);
        Ok(CoreStack {
            core,
            engine,
            agg,
            gateways,
            writer,
            bridges,
            status,
            status_task,
        })
    }

    /// FIX session table of the in-process gateways (admin API `/v1/lp/sessions`).
    pub fn lp_status(&self) -> std::sync::Arc<std::sync::RwLock<Vec<fix_gateway::SessionStatus>>> {
        self.status.clone()
    }

    /// Stops the bridge, the FIX sessions and the engine (final snapshot).
    pub async fn shutdown(self) {
        for b in self.bridges {
            b.abort();
            let _ = b.await;
        }
        if let Some(t) = self.status_task {
            t.abort();
        }
        for g in self.gateways {
            g.shutdown().await;
        }
        self.engine.shutdown();
        let _ = tokio::task::spawn_blocking(move || self.writer.join()).await;
    }
}

#[cfg(test)]
mod spec_tests {
    use super::*;

    #[test]
    fn fx_and_metal_specs() {
        let px = |s: &str| s.parse::<domain::Price>().unwrap();
        let eur = fx_spec("EUR/USD", px("0.00001")).unwrap();
        assert_eq!((eur.contract_size, eur.digits), (100_000, 5));
        assert_eq!(eur.asset_class, AssetClass::MajorFx);
        assert_eq!(
            fx_spec("EUR/TRY", px("0.00001")).unwrap().asset_class,
            AssetClass::MinorFx
        );
        let xau = fx_spec("XAU/USD", px("0.01")).unwrap();
        assert_eq!(
            (xau.symbol.as_str(), xau.contract_size, xau.digits),
            ("XAUUSD", 100, 2)
        );
        assert_eq!(xau.asset_class, AssetClass::Gold);
        let xag = fx_spec("XAG/USD", px("0.001")).unwrap();
        assert_eq!(
            (xag.contract_size, xag.asset_class),
            (5_000, AssetClass::Commodity)
        );
        // LMAX contracts: gold 10 oz, silver 500 oz -> 10 contracts per lot.
        assert_eq!(lot_units("XAU/USD") / 10, 10);
        assert_eq!(lot_units("XAG/USD") / 500, 10);
        // Minis / indices are not mapped.
        assert!(fx_spec("XAU/USDm", px("0.01")).is_none());
        assert!(fx_spec("AUS200", px("0.1")).is_none());
    }
}

#[cfg(test)]
mod lmax_list_tests {
    use super::*;

    /// Every line of the shipped LMAX list maps to an engine symbol, divides a
    /// lot exactly, and 0.01 lot meets LMAX's minimum order size
    /// (0.1 contract; 1 contract for platinum / palladium).
    #[test]
    fn shipped_lmax_instruments_are_tradable() {
        let list = include_str!("../../../deploy/lmax-demo/lmax-instruments.txt");
        let mut n = 0;
        for line in list.lines().filter(|l| !l.trim().is_empty()) {
            let f: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(f.len(), 4, "{line}");
            let (sym, tick, cs) = (f[0], f[2], f[3].parse::<i64>().unwrap());
            let spec =
                fx_spec(sym, tick.parse().unwrap()).unwrap_or_else(|| panic!("no spec: {line}"));
            let lot = lot_units(sym);
            assert_eq!(spec.contract_size, lot, "{line}");
            assert_eq!(lot % cs, 0, "contract does not divide a lot: {line}");
            let per_lot = lot / cs; // LP OrderQty of 1 lot
            let min_lp = per_lot as f64 / 100.0; // OrderQty of 0.01 lot
            let lmax_min = if sym.starts_with("XPT") || sym.starts_with("XPD") {
                1.0
            } else {
                0.1
            };
            assert!(
                min_lp >= lmax_min - 1e-9,
                "0.01 lot = {min_lp} < LMAX min {lmax_min}: {line}"
            );
            n += 1;
        }
        assert_eq!(n, 91);
    }
}
