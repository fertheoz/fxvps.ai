use fix_gateway::{GatewayConfig, GatewayEvent};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "services/fix-gateway/config/default.toml".into());
    let cfg = GatewayConfig::load(&path)?;
    #[cfg(feature = "nats")]
    let nats_cfg = cfg.nats.clone();
    #[cfg(not(feature = "nats"))]
    if cfg.nats.is_some() {
        tracing::warn!("[nats] configured but binary built without the `nats` feature");
    }
    let gw = fix_gateway::start(cfg)?;
    if let Some(addr) = std::env::var("FIX_STATUS_ADDR")
        .ok()
        .filter(|v| !v.is_empty())
    {
        let local = fix_gateway::status_http::spawn(&addr, gw.status_source()).await?;
        tracing::info!(%local, "session status endpoint: GET /status");
    }
    #[cfg(feature = "nats")]
    if let Some(n) = nats_cfg {
        let rx = gw.subscribe();
        tokio::spawn(async move {
            if let Err(e) = fix_gateway::nats::run_publisher(n, rx).await {
                tracing::error!(error = %e, "NATS publisher stopped");
            }
        });
    }
    let mut rx = gw.subscribe();
    let log = tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            match ev {
                GatewayEvent::Quote(q) => tracing::debug!(
                    symbol = %q.symbol,
                    bid = ?q.best_bid().map(|l| l.price.to_string()),
                    ask = ?q.best_ask().map(|l| l.price.to_string()),
                    "quote"
                ),
                other => tracing::info!(event = ?other, "gateway event"),
            }
        }
    });
    tokio::signal::ctrl_c().await?;
    gw.shutdown().await;
    log.abort();
    Ok(())
}
