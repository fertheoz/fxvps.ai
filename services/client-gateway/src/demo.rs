//! `--demo`: lp-simulator + fix-gateway + core-engine in-process, so the WebSocket
//! server streams live (group marked-up) quotes and orders go through real risk
//! checks, the OMS and the ledger before reaching the simulated LP.
//!
//! Seeded accounts: `DEMO-1`..`DEMO-3`, each funded with `cfg.demo_balance`
//! `cfg.currency` in group `demo-retail` (ESMA retail 1:30 on major FX,
//! netting, A-book, 0.5 pip markup, USD 3.50 commission per lot per side).

use std::path::PathBuf;
use std::sync::Arc;

use core_engine::api::CoreApi;
use core_engine::stack::{CoreStack, Seed, StackConfig};
use fix_gateway::GatewayConfig;
use lp_simulator::{SimConfig, SimHandle};
use money::{Currency, Money};

use crate::auth::Authenticator;
use crate::config::ClientGatewayConfig;
use crate::hub::Hub;

const SIM_TOML: &str = include_str!("../../lp-simulator/config/default.toml");
const GW_TOML: &str = include_str!("../../fix-gateway/config/default.toml");

/// Number of seeded demo accounts.
pub const DEMO_ACCOUNTS: u64 = 3;

pub struct Demo {
    pub hub: Arc<Hub>,
    pub sim: SimHandle,
    pub stack: CoreStack,
}

#[derive(Clone, Debug, Default)]
pub struct DemoOptions {
    /// Simulator tick interval override.
    pub tick_ms: Option<u64>,
    /// Simulator random-walk step override (ticks).
    pub volatility_ticks: Option<i64>,
    /// Engine journal directory; a fresh temp directory if `None`.
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    #[error("simulator config: {0}")]
    SimConfig(#[from] lp_simulator::config::ConfigError),
    #[error("gateway config: {0}")]
    GwConfig(#[from] fix_gateway::config::ConfigError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("core stack: {0}")]
    Stack(#[from] core_engine::stack::StackError),
    #[error("demo account funding: {0}")]
    Funding(String),
}

/// Seed for `n` demo accounts funded per the gateway config.
pub fn demo_seed(cfg: &ClientGatewayConfig, gw: &GatewayConfig) -> Result<Seed, DemoError> {
    let ccy: Currency = cfg
        .currency
        .parse()
        .map_err(|_| DemoError::Funding(format!("bad currency {}", cfg.currency)))?;
    let deposit =
        Money::parse(&cfg.demo_balance, ccy).map_err(|e| DemoError::Funding(e.to_string()))?;
    Ok(Seed::demo(gw, DEMO_ACCOUNTS, deposit))
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "fxvps-demo-{}-{}",
        std::process::id(),
        domain::now_ns()
    ))
}

impl Demo {
    /// Starts the simulator on ephemeral ports, the FIX gateway against it, the
    /// core engine and the hub bridge. `tick_ms` overrides the simulator tick.
    pub async fn start(
        cfg: ClientGatewayConfig,
        auth: Authenticator,
        tick_ms: Option<u64>,
    ) -> Result<Demo, DemoError> {
        Demo::start_with(
            cfg,
            auth,
            DemoOptions {
                tick_ms,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn start_with(
        cfg: ClientGatewayConfig,
        auth: Authenticator,
        opts: DemoOptions,
    ) -> Result<Demo, DemoError> {
        let mut sim_cfg = SimConfig::from_toml(SIM_TOML)?;
        sim_cfg.md.listen = "127.0.0.1:0".into();
        sim_cfg.trade.listen = "127.0.0.1:0".into();
        if let Some(t) = opts.tick_ms {
            sim_cfg.tick_interval_ms = t;
        }
        if let Some(v) = opts.volatility_ticks {
            for i in &mut sim_cfg.instruments {
                i.volatility_ticks = v;
            }
        }
        let sim = lp_simulator::start(sim_cfg).await?;
        let mut gw_cfg = GatewayConfig::from_toml(GW_TOML)?;
        gw_cfg.lp = "SIM".into();
        gw_cfg.md.addr = sim.md_addr.to_string();
        gw_cfg.trade.addr = sim.trade_addr.to_string();
        gw_cfg.reconnect_delay_ms = 200;
        gw_cfg.store_dir = None;
        gw_cfg.nats = None;
        let seed = demo_seed(&cfg, &gw_cfg)?;
        let instruments = gw_cfg.instruments.clone();
        let mut st = StackConfig::new(gw_cfg, opts.data_dir.unwrap_or_else(temp_dir));
        st.seed = Some(seed);
        let stack = CoreStack::start(st).await?;
        let core: Arc<dyn CoreApi> = stack.core.clone();
        let hub = Hub::with_instruments(cfg, auth, &instruments, Some(core.clone()));
        tokio::spawn(hub.clone().run_core_bridge(core.subscribe()));
        Ok(Demo { hub, sim, stack })
    }

    pub async fn shutdown(self) {
        self.stack.shutdown().await;
        self.sim.shutdown().await;
    }
}
