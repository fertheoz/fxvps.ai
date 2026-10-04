use std::sync::Arc;

use client_gateway::auth::{issue_hs256, Authenticator, DEV_HS256_SECRET};
use client_gateway::demo::Demo;
use client_gateway::{ClientGatewayConfig, Hub};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: client-gateway [--demo] [--config client-gateway.toml] \
[--fix-config fix-gateway.toml] [--listen ADDR]";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let mut demo = false;
    let mut cfg_path = None;
    let mut fix_cfg = None;
    let mut listen = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => demo = true,
            "--config" => cfg_path = args.next(),
            "--fix-config" => fix_cfg = args.next(),
            "--listen" => listen = args.next(),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}").into()),
        }
    }
    let mut cfg = match cfg_path {
        Some(p) => ClientGatewayConfig::load(p)?,
        None => ClientGatewayConfig::default(),
    };
    if let Some(l) = listen {
        cfg.listen = l;
    }
    let auth = Authenticator::from_env()?;
    if auth.dev_key {
        tracing::warn!(
            "using the built-in DEVELOPMENT JWT key; set FXVPS_JWT_HS256_SECRET or FXVPS_JWT_JWKS_FILE outside local dev"
        );
    }
    let dev_key = auth.dev_key;
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    let addr = listener.local_addr()?;

    let mut demo_handle = None;
    let mut fix_handle = None;
    let hub: Arc<Hub> = if demo {
        let d = Demo::start(cfg, auth, None).await?;
        let hub = d.hub.clone();
        demo_handle = Some(d);
        hub
    } else if let Some(p) = fix_cfg {
        let gw_cfg = fix_gateway::GatewayConfig::load(p)?;
        let symbols: Vec<String> = gw_cfg
            .instruments
            .iter()
            .map(|i| i.symbol.clone())
            .collect();
        let gw = fix_gateway::start(gw_cfg)?;
        let hub = Hub::new(cfg, auth, symbols, Some(gw.orders()));
        tokio::spawn(hub.clone().run_bridge(gw.subscribe()));
        fix_handle = Some(gw);
        hub
    } else {
        tracing::warn!("no --demo / --fix-config: serving without a market data feed");
        Hub::new(cfg, auth, Vec::new(), None)
    };

    tracing::info!(%addr, "client-gateway listening (ws://{addr}/ws, /healthz, /metrics)");
    if demo && dev_key {
        // Dev key only: a convenience token for local clients.
        let t = issue_hs256(
            DEV_HS256_SECRET.as_bytes(),
            "demo-user",
            &["DEMO-1"],
            24 * 3600,
        );
        tracing::info!("demo token (account DEMO-1, dev key): {t}");
    }
    tokio::select! {
        r = client_gateway::serve(hub, listener) => r?,
        _ = tokio::signal::ctrl_c() => {}
    }
    if let Some(d) = demo_handle {
        d.shutdown().await;
    }
    if let Some(g) = fix_handle {
        g.shutdown().await;
    }
    Ok(())
}
