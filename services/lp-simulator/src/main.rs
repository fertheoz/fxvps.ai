use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "services/lp-simulator/config/default.toml".into());
    let cfg = lp_simulator::SimConfig::load(&path)?;
    let sim = lp_simulator::start(cfg).await?;
    tracing::info!(md = %sim.md_addr, trade = %sim.trade_addr, "ready; Ctrl-C to stop");
    tokio::signal::ctrl_c().await?;
    sim.shutdown().await;
    Ok(())
}
