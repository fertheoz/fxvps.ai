use fix_gateway::{GatewayConfig, GatewayEvent};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if let Some(path) = std::env::var("FIX_CONFIG_FILE")
        .ok()
        .filter(|v| !v.is_empty())
    {
        return managed_mode(path).await;
    }
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

/// `FIX_CONFIG_FILE` set: LP settings come from the admin endpoint (`PUT /config`,
/// token `FIX_ADMIN_TOKEN`) and every update restarts the FIX sessions.
async fn managed_mode(path: String) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, RwLock};

    let managed = Arc::new(fix_gateway::managed::Managed::open(&path)?);
    let table: Arc<RwLock<Vec<fix_gateway::SessionStatus>>> = Arc::default();
    let token = std::env::var("FIX_ADMIN_TOKEN")
        .ok()
        .filter(|t| t.len() >= 16)
        .ok_or("managed mode needs FIX_ADMIN_TOKEN (at least 16 characters)")?;
    let addr = std::env::var("FIX_STATUS_ADDR").unwrap_or_else(|_| "127.0.0.1:9890".into());
    let admin = fix_gateway::status_http::Admin {
        token,
        managed: managed.clone(),
    };
    let local =
        fix_gateway::status_http::spawn_with_admin(&addr, table.clone(), Some(admin)).await?;
    tracing::info!(%local, config = %path, "managed mode: GET /status, GET|PUT /config");

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        let Some(cfg) = managed.current() else {
            tracing::info!("no LP configured yet; waiting for PUT /config");
            if let Ok(mut t) = table.write() {
                t.clear();
            }
            tokio::select! {
                _ = managed.changed() => continue,
                _ = &mut ctrl_c => return Ok(()),
            }
        };
        let gw = match fix_gateway::start(cfg) {
            Ok(gw) => gw,
            Err(e) => {
                tracing::error!(error = %e, "cannot start gateway with stored config");
                tokio::select! {
                    _ = managed.changed() => continue,
                    _ = &mut ctrl_c => return Ok(()),
                }
            }
        };
        let mut rx = gw.subscribe();
        let src = gw.status_source();
        let mirror = {
            let table = table.clone();
            tokio::spawn(async move {
                loop {
                    if let (Ok(s), Ok(mut t)) = (src.read(), table.write()) {
                        *t = s.clone();
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            })
        };
        let log = tokio::spawn(async move {
            while let Ok(ev) = rx.recv().await {
                if !matches!(ev, GatewayEvent::Quote(_)) {
                    tracing::info!(event = ?ev, "gateway event");
                }
            }
        });
        let stop = tokio::select! {
            _ = managed.changed() => false,
            _ = &mut ctrl_c => true,
        };
        gw.shutdown().await;
        mirror.abort();
        log.abort();
        if stop {
            return Ok(());
        }
        tracing::info!("LP config changed; restarting sessions");
    }
}
