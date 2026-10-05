use core_engine::admin::{self, auth::Authenticator, AdminConfig};
use core_engine::{spawn, Settings};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let dir = std::env::var("CORE_DATA_DIR").unwrap_or_else(|_| "./data/core-engine".into());
    let addr = std::env::var("CORE_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:8090".into());
    let flag = |k: &str| std::env::var(k).is_ok_and(|v| v == "1" || v == "true");
    let mut settings = Settings::new(&dir);
    if let Some(n) = std::env::var("CORE_SNAPSHOT_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        settings.snapshot_every = n;
    }
    let auth = Authenticator::from_env(flag("CORE_DEV_AUTH"), &addr).map_err(|e| e.0)?;
    let (handle, join) = spawn(settings)?;
    if flag("CORE_SEED") && admin::seed::seed_if_empty(&handle).await? {
        tracing::info!("seeded demo data");
    }
    let mut admin_cfg = AdminConfig::new(&dir).with_env(flag("CORE_DEV_AUTH"))?;
    if let Some(url) = std::env::var("CORE_LP_STATUS_URL")
        .ok()
        .filter(|v| !v.is_empty())
    {
        tracing::info!(%url, "polling fix-gateway session status");
        admin_cfg.lp_status = Some(admin::lp_poll::spawn(
            url,
            std::time::Duration::from_secs(2),
        ));
    }
    let app = admin::app(handle.clone(), auth, admin_cfg)?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("core-engine admin API on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    handle.shutdown();
    let _ = join.join();
    Ok(())
}
