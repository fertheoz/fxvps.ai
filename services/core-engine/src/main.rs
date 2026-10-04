use core_engine::{router, spawn, Settings};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let dir = std::env::var("CORE_DATA_DIR").unwrap_or_else(|_| "./data/core-engine".into());
    let addr = std::env::var("CORE_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:8090".into());
    let mut settings = Settings::new(dir);
    if let Some(n) = std::env::var("CORE_SNAPSHOT_EVERY")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        settings.snapshot_every = n;
    }
    let (handle, join) = spawn(settings)?;
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("core-engine admin API on {addr}");
    axum::serve(listener, router(handle.clone()))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    handle.shutdown();
    let _ = join.join();
    Ok(())
}
