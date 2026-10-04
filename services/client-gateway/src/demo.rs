//! `--demo`: lp-simulator + fix-gateway in-process, bridged into the hub, so the
//! WebSocket server streams live simulated quotes and routes orders to the simulator.

use std::sync::Arc;

use fix_gateway::{GatewayConfig, GatewayHandle};
use lp_simulator::{SimConfig, SimHandle};

use crate::auth::Authenticator;
use crate::config::ClientGatewayConfig;
use crate::hub::Hub;

const SIM_TOML: &str = include_str!("../../lp-simulator/config/default.toml");
const GW_TOML: &str = include_str!("../../fix-gateway/config/default.toml");

pub struct Demo {
    pub hub: Arc<Hub>,
    pub sim: SimHandle,
    pub gateway: GatewayHandle,
}

#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    #[error("simulator config: {0}")]
    SimConfig(#[from] lp_simulator::config::ConfigError),
    #[error("gateway config: {0}")]
    GwConfig(#[from] fix_gateway::config::ConfigError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("gateway: {0}")]
    Gateway(#[from] fix_gateway::GatewayError),
}

impl Demo {
    /// Starts the simulator on ephemeral ports, the FIX gateway against it and the hub
    /// bridge. `tick_ms` overrides the simulator tick interval.
    pub async fn start(
        cfg: ClientGatewayConfig,
        auth: Authenticator,
        tick_ms: Option<u64>,
    ) -> Result<Demo, DemoError> {
        let mut sim_cfg = SimConfig::from_toml(SIM_TOML)?;
        sim_cfg.md.listen = "127.0.0.1:0".into();
        sim_cfg.trade.listen = "127.0.0.1:0".into();
        if let Some(t) = tick_ms {
            sim_cfg.tick_interval_ms = t;
        }
        let sim = lp_simulator::start(sim_cfg).await?;
        let mut gw_cfg = GatewayConfig::from_toml(GW_TOML)?;
        gw_cfg.lp = "SIM".into();
        gw_cfg.md.addr = sim.md_addr.to_string();
        gw_cfg.trade.addr = sim.trade_addr.to_string();
        gw_cfg.reconnect_delay_ms = 200;
        gw_cfg.store_dir = None;
        gw_cfg.nats = None;
        let symbols: Vec<String> = gw_cfg
            .instruments
            .iter()
            .map(|i| i.symbol.clone())
            .collect();
        let gateway = fix_gateway::start(gw_cfg)?;
        let hub = Hub::new(cfg, auth, symbols, Some(gateway.orders()));
        tokio::spawn(hub.clone().run_bridge(gateway.subscribe()));
        Ok(Demo { hub, sim, gateway })
    }

    pub async fn shutdown(self) {
        self.gateway.shutdown().await;
        self.sim.shutdown().await;
    }
}
