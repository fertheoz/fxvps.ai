//! NATS publisher (feature `nats`). Subjects:
//! `<prefix>.quote.<SYMBOL>`, `<prefix>.exec.<SYMBOL>`, `<prefix>.event` (everything else).
//! Payload: JSON (`Fixed` values as decimal strings).

use tokio::sync::broadcast;
use tracing::warn;

use crate::config::NatsConfig;
use crate::gateway::GatewayEvent;

fn token(symbol: &str) -> String {
    symbol
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Subject for an event.
pub fn subject(prefix: &str, ev: &GatewayEvent) -> String {
    match ev {
        GatewayEvent::Quote(q) => format!("{prefix}.quote.{}", token(&q.symbol)),
        GatewayEvent::Execution(e) => format!("{prefix}.exec.{}", token(&e.symbol)),
        _ => format!("{prefix}.event"),
    }
}

/// Publishes every gateway event until the channel closes.
pub async fn run_publisher(
    cfg: NatsConfig,
    mut rx: broadcast::Receiver<GatewayEvent>,
) -> Result<(), async_nats::ConnectError> {
    let client = async_nats::connect(&cfg.url).await?;
    loop {
        match rx.recv().await {
            Ok(ev) => {
                let payload = match serde_json::to_vec(&ev) {
                    Ok(p) => p,
                    Err(e) => {
                        warn!(error = %e, "serialize event");
                        continue;
                    }
                };
                if let Err(e) = client
                    .publish(subject(&cfg.subject_prefix, &ev), payload.into())
                    .await
                {
                    warn!(error = %e, "NATS publish failed");
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => warn!(n, "NATS publisher lagged"),
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    Ok(())
}
