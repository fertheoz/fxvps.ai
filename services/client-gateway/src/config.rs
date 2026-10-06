//! Client gateway configuration (TOML file and/or defaults).

use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ClientGatewayConfig {
    /// HTTP/WebSocket listen address.
    pub listen: String,
    /// Default and maximum quote rate per connection (Hz). Clients may ask for less.
    pub max_quote_hz: u32,
    /// Bounded outbound queue per connection (frames). Full queue = slow consumer.
    pub queue_capacity: usize,
    /// Sustained order commands per second per account.
    pub orders_per_second: u32,
    /// Burst size for order commands per account.
    pub order_burst: u32,
    /// Time allowed for Hello + Auth after connect.
    pub auth_timeout_ms: u64,
    /// Server heartbeat interval.
    pub heartbeat_secs: u64,
    /// Max inbound frame size in bytes.
    pub max_frame_bytes: usize,
    /// Candles retained per symbol and timeframe.
    pub candle_capacity: usize,
    /// Initial deposit of each seeded demo account (account currency units).
    pub demo_balance: String,
    /// Currency of seeded demo accounts.
    pub currency: String,
    /// Max concurrent WebSocket connections (0 = unlimited).
    pub max_connections: usize,
    /// Max concurrent connections per client IP (0 = unlimited).
    pub max_connections_per_ip: usize,
    /// Max concurrent authenticated connections per token subject (0 = unlimited).
    pub max_connections_per_subject: usize,
    /// Serve `/metrics` on this separate address instead of the public listener
    /// (e.g. `0.0.0.0:9090`, reachable only by Prometheus via NetworkPolicy).
    pub metrics_listen: Option<String>,
    /// Admin API base (`http://127.0.0.1:8090`) whose `/v1/client/*` routes are
    /// exposed to clients as `/api/client/*` on this listener; `None` = no self-service.
    #[serde(default)]
    pub client_api_upstream: Option<String>,
    /// Allowed WebSocket `Origin`s (empty = any; native clients send none).
    pub allowed_origins: Vec<String>,
}

impl Default for ClientGatewayConfig {
    fn default() -> Self {
        ClientGatewayConfig {
            listen: "127.0.0.1:8080".into(),
            max_quote_hz: 10,
            queue_capacity: 256,
            orders_per_second: 10,
            order_burst: 20,
            auth_timeout_ms: 5_000,
            heartbeat_secs: 15,
            max_frame_bytes: 64 * 1024,
            candle_capacity: 1_000,
            demo_balance: "10000".into(),
            currency: "USD".into(),
            max_connections: 10_000,
            max_connections_per_ip: 50,
            max_connections_per_subject: 20,
            metrics_listen: None,
            client_api_upstream: None,
            allowed_origins: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

impl ClientGatewayConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let c: Self = toml::from_str(&std::fs::read_to_string(path)?)?;
        c.validate()?;
        Ok(c)
    }

    /// Environment overrides (container deployments without a config file):
    /// `FXVPS_METRICS_LISTEN`, `FXVPS_ALLOWED_ORIGINS` (comma separated),
    /// `FXVPS_MAX_CONNECTIONS`, `FXVPS_MAX_CONNECTIONS_PER_IP`,
    /// `FXVPS_MAX_CONNECTIONS_PER_SUBJECT`.
    pub fn apply_env(&mut self) -> Result<(), ConfigError> {
        self.apply_vars(|k| std::env::var(k).ok())
    }

    fn apply_vars(&mut self, var: impl Fn(&str) -> Option<String>) -> Result<(), ConfigError> {
        let var = |k: &str| var(k).filter(|v| !v.trim().is_empty());
        let num = |k: &str, cur: usize| -> Result<usize, ConfigError> {
            match var(k) {
                Some(v) => v
                    .trim()
                    .parse()
                    .map_err(|_| ConfigError::Invalid(format!("{k} must be a number"))),
                None => Ok(cur),
            }
        };
        if let Some(v) = var("FXVPS_METRICS_LISTEN") {
            self.metrics_listen = Some(v);
        }
        if let Some(v) = var("FXVPS_CLIENT_API_UPSTREAM")
            .or_else(|| var("CORE_ADMIN_ADDR").map(|a| format!("http://{a}")))
        {
            self.client_api_upstream = Some(v.trim_end_matches('/').to_string());
        }
        if let Some(v) = var("FXVPS_ALLOWED_ORIGINS") {
            self.allowed_origins = v
                .split(',')
                .map(|s| s.trim().trim_end_matches('/').to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        self.max_connections = num("FXVPS_MAX_CONNECTIONS", self.max_connections)?;
        self.max_connections_per_ip =
            num("FXVPS_MAX_CONNECTIONS_PER_IP", self.max_connections_per_ip)?;
        self.max_connections_per_subject = num(
            "FXVPS_MAX_CONNECTIONS_PER_SUBJECT",
            self.max_connections_per_subject,
        )?;
        // Per-account order rate (load tests raise it on an isolated instance).
        self.orders_per_second =
            num("FXVPS_ORDERS_PER_SECOND", self.orders_per_second as usize)? as u32;
        self.order_burst = num("FXVPS_ORDER_BURST", self.order_burst as usize)? as u32;
        // Seeded demo deposit (load tests: margin must not be what runs out).
        if let Some(v) = var("FXVPS_DEMO_BALANCE") {
            self.demo_balance = v.trim().to_string();
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_quote_hz == 0 || self.max_quote_hz > 1000 {
            return Err(ConfigError::Invalid("max_quote_hz must be 1..=1000".into()));
        }
        if self.queue_capacity == 0 || self.orders_per_second == 0 || self.order_burst == 0 {
            return Err(ConfigError::Invalid(
                "queue_capacity, orders_per_second, order_burst must be > 0".into(),
            ));
        }
        if self.demo_balance.parse::<domain::Fixed>().is_err() {
            return Err(ConfigError::Invalid("demo_balance is not a decimal".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_overrides() {
        let mut c = ClientGatewayConfig::default();
        let env = |k: &str| match k {
            "FXVPS_METRICS_LISTEN" => Some("0.0.0.0:9090".to_string()),
            "FXVPS_ALLOWED_ORIGINS" => Some("https://a.test/, https://b.test".to_string()),
            "FXVPS_MAX_CONNECTIONS_PER_IP" => Some("7".to_string()),
            _ => None,
        };
        c.apply_vars(env).unwrap();
        assert_eq!(c.metrics_listen.as_deref(), Some("0.0.0.0:9090"));
        assert_eq!(c.allowed_origins, vec!["https://a.test", "https://b.test"]);
        assert_eq!(c.max_connections_per_ip, 7);
        assert_eq!(c.max_connections, 10_000);
        let bad = |k: &str| (k == "FXVPS_MAX_CONNECTIONS").then(|| "lots".to_string());
        assert!(c.apply_vars(bad).is_err());
    }
}
