//! FIX gateway: initiator towards an LP (lp-simulator by default) with separate MD and
//! trading sessions. Market data is normalized into [`domain::Quote`], execution reports
//! into [`domain::Execution`], published on a tokio broadcast channel (and NATS with the
//! `nats` feature). Internal components submit [`OrderCommand`]s.

pub mod audit;
pub mod config;
pub mod fixlog;
pub mod gateway;
pub mod managed;
pub mod normalize;
pub mod status_http;
pub mod tls;

#[cfg(feature = "nats")]
pub mod nats;

pub mod hours;
pub use config::{GatewayConfig, NatsConfig, SessionEndpoint, TlsEndpoint};
pub use gateway::{
    apply_status, start, GatewayError, GatewayEvent, GatewayHandle, OrderCommand, SessionKind,
    SessionStatus,
};
pub use hours::SessionHours;
