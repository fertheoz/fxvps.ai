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
    /// Demo balance shown in account snapshots (account currency units).
    pub demo_balance: String,
    pub currency: String,
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
            demo_balance: "100000".into(),
            currency: "USD".into(),
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
