use std::path::{Path, PathBuf};

use domain::Instrument;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// LP name stamped on normalized events.
    #[serde(default = "default_lp")]
    pub lp: String,
    #[serde(default = "default_hb")]
    pub heartbeat_secs: u64,
    #[serde(default = "default_depth")]
    pub market_depth: u64,
    #[serde(default = "default_reconnect")]
    pub reconnect_delay_ms: u64,
    /// Failed logons in a row after which a session stops retrying (protects
    /// the LP account from lock-outs on wrong credentials); 0 = never stop.
    #[serde(default = "default_max_logon_failures")]
    pub max_logon_failures: u32,
    /// Orders per second sent on the trading session (LMAX allows 100/s;
    /// default 80 leaves headroom). 0 = no brake.
    #[serde(default = "default_max_orders_per_sec")]
    pub max_orders_per_sec: u32,
    /// false: configured but paused, no connection attempts at all.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// SecurityIDSource(22). Assumption for LMAX: `8`.
    #[serde(default = "default_source")]
    pub security_id_source: String,
    /// Directory for persistent FIX stores (`<dir>/md`, `<dir>/trade`); memory if absent.
    pub store_dir: Option<PathBuf>,
    pub md: SessionEndpoint,
    pub trade: SessionEndpoint,
    pub instruments: Vec<Instrument>,
    pub nats: Option<NatsConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionEndpoint {
    pub addr: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default = "yes")]
    pub reset_on_logon: bool,
    /// TLS towards the LP; absent = plain TCP (simulator, cross-connect).
    #[serde(default)]
    pub tls: Option<TlsEndpoint>,
}

/// `[md.tls]` / `[trade.tls]`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TlsEndpoint {
    /// Name checked against the certificate; default: host part of `addr`.
    pub server_name: Option<String>,
    /// PEM CA bundle; default: Mozilla roots (webpki-roots).
    pub ca_file: Option<PathBuf>,
    /// PEM client certificate chain + key, if the LP requires mutual TLS.
    pub client_cert_file: Option<PathBuf>,
    pub client_key_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NatsConfig {
    pub url: String,
    pub subject_prefix: String,
}

fn default_lp() -> String {
    "LMAX".into()
}
fn default_hb() -> u64 {
    30
}
fn default_depth() -> u64 {
    5
}
fn default_max_logon_failures() -> u32 {
    3
}
fn default_reconnect() -> u64 {
    1000
}
fn default_source() -> String {
    "8".into()
}
fn default_max_orders_per_sec() -> u32 {
    80
}

fn yes() -> bool {
    true
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("read config: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse config: {0}")]
    Toml(#[from] toml::de::Error),
}

impl GatewayConfig {
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(s)?)
    }

    /// Loads a TOML file and applies password overrides from the environment.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let mut cfg = Self::from_toml(&std::fs::read_to_string(path)?)?;
        if let Ok(p) = std::env::var("FIX_GATEWAY_MD_PASSWORD") {
            cfg.md.password = Some(p);
        }
        if let Ok(p) = std::env::var("FIX_GATEWAY_TRADE_PASSWORD") {
            cfg.trade.password = Some(p);
        }
        Ok(cfg)
    }

    pub fn instrument_by_symbol(&self, symbol: &str) -> Option<&Instrument> {
        self.instruments.iter().find(|i| i.symbol == symbol)
    }

    pub fn instrument_by_security_id(&self, id: &str) -> Option<&Instrument> {
        self.instruments.iter().find(|i| i.security_id == id)
    }
}
