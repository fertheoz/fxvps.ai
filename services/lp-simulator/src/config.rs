use std::path::Path;

use domain::{Fixed, Price, Qty};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct SimConfig {
    /// Scenario control HTTP (`GET /state`, `POST /shock`, `POST /scenario`); loopback only. None = off.
    #[serde(default)]
    pub control_listen: Option<String>,
    #[serde(default = "default_tick")]
    pub tick_interval_ms: u64,
    #[serde(default = "default_depth")]
    pub depth: usize,
    /// RNG seed for a reproducible price path; random if absent.
    pub seed: Option<u64>,
    #[serde(default = "default_source")]
    pub security_id_source: String,
    pub md: EndpointConfig,
    pub trade: EndpointConfig,
    pub instruments: Vec<SimInstrument>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EndpointConfig {
    /// `host:port`; port 0 picks a free port (tests).
    pub listen: String,
    /// Our SenderCompID.
    pub comp_id: String,
    /// Expected client SenderCompID.
    pub client_comp_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SimInstrument {
    pub symbol: String,
    pub security_id: String,
    pub initial_mid: Price,
    pub tick_size: Price,
    pub spread_ticks: i64,
    pub volatility_ticks: i64,
    /// Size of the best level; level `i` carries `(i + 1) * level_size`.
    pub level_size: Qty,
    /// Pull towards `initial_mid` per step (ticks); 0 = pure random walk.
    /// Keeps a simulated LP near a real one when both feed the aggregator.
    #[serde(default)]
    pub revert_ticks: i64,
}

fn default_tick() -> u64 {
    250
}
fn default_depth() -> usize {
    5
}
fn default_source() -> String {
    "8".into()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("read config: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse config: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

impl SimConfig {
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        let cfg: SimConfig = toml::from_str(s)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.depth == 0 {
            return Err(ConfigError::Invalid("depth must be >= 1".into()));
        }
        for i in &self.instruments {
            if !i.tick_size.is_positive() || !i.initial_mid.is_positive() || i.spread_ticks < 1 {
                return Err(ConfigError::Invalid(format!(
                    "instrument {}: bad prices",
                    i.symbol
                )));
            }
            if !i.level_size.is_positive() {
                return Err(ConfigError::Invalid(format!(
                    "instrument {}: bad level_size",
                    i.symbol
                )));
            }
        }
        Ok(())
    }

    /// Small config for tests: ephemeral ports, EUR/USD only.
    pub fn for_tests() -> Self {
        let ep = |comp: &str, client: &str| EndpointConfig {
            listen: "127.0.0.1:0".into(),
            comp_id: comp.into(),
            client_comp_id: client.into(),
            username: Some("demo".into()),
            password: Some("demo".into()),
        };
        SimConfig {
            tick_interval_ms: 20,
            depth: 3,
            seed: Some(7),
            security_id_source: "8".into(),
            control_listen: None,
            md: ep("LMXBDM", "FXVPS-MD"),
            trade: ep("LMXBD", "FXVPS-TRD"),
            instruments: vec![SimInstrument {
                symbol: "EUR/USD".into(),
                security_id: "4001".into(),
                initial_mid: Fixed::from_parts(108_500, 5),
                tick_size: Fixed::from_parts(1, 5),
                spread_ticks: 2,
                volatility_ticks: 2,
                level_size: Fixed::from_int(1_000_000),
                revert_ticks: 0,
            }],
        }
    }
}
