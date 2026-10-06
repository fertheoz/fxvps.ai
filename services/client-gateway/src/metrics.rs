//! Prometheus metrics served on `/metrics`.

use prometheus::{Encoder, IntCounter, IntGauge, Registry, TextEncoder};

pub struct Metrics {
    registry: Registry,
    pub connections: IntGauge,
    pub auth_failures: IntCounter,
    pub connections_rejected: IntCounter,
    pub sessions_expired: IntCounter,
    pub quote_batches_sent: IntCounter,
    pub quotes_sent: IntCounter,
    pub quotes_conflated: IntCounter,
    pub depth_frames_sent: IntCounter,
    pub slow_consumer_drops: IntCounter,
    pub orders_received: IntCounter,
    pub orders_rate_limited: IntCounter,
    pub frames_in: IntCounter,
}

fn counter(r: &Registry, name: &str, help: &str) -> IntCounter {
    let c = IntCounter::new(name, help).expect("valid metric");
    r.register(Box::new(c.clone())).expect("unique metric");
    c
}

impl Default for Metrics {
    fn default() -> Self {
        let registry = Registry::new();
        let connections =
            IntGauge::new("client_gw_connections", "Open WebSocket connections").expect("metric");
        registry
            .register(Box::new(connections.clone()))
            .expect("unique metric");
        let r = &registry;
        Metrics {
            connections,
            connections_rejected: counter(
                r,
                "client_gw_connections_rejected_total",
                "Connections refused by the global / per-IP / per-subject caps",
            ),
            sessions_expired: counter(
                r,
                "client_gw_sessions_expired_total",
                "Connections closed because the token expired",
            ),
            auth_failures: counter(
                r,
                "client_gw_auth_failures_total",
                "Rejected authentications",
            ),
            quote_batches_sent: counter(
                r,
                "client_gw_quote_batches_sent_total",
                "QuoteBatch frames",
            ),
            quotes_sent: counter(r, "client_gw_quotes_sent_total", "Quotes delivered"),
            quotes_conflated: counter(
                r,
                "client_gw_quotes_conflated_total",
                "Quotes replaced by a newer one before delivery",
            ),
            depth_frames_sent: counter(r, "client_gw_depth_frames_sent_total", "Depth frames"),
            slow_consumer_drops: counter(
                r,
                "client_gw_slow_consumer_drops_total",
                "Connections closed because the outbound queue was full",
            ),
            orders_received: counter(r, "client_gw_orders_received_total", "Order commands"),
            orders_rate_limited: counter(
                r,
                "client_gw_orders_rate_limited_total",
                "Order commands rejected by the per-account rate limit",
            ),
            frames_in: counter(r, "client_gw_frames_in_total", "Inbound frames"),
            registry,
        }
    }
}

impl Metrics {
    pub fn render(&self) -> String {
        let mut buf = Vec::new();
        let _ = TextEncoder::new().encode(&self.registry.gather(), &mut buf);
        String::from_utf8(buf).unwrap_or_default()
    }
}
