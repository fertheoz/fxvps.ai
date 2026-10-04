use std::sync::Arc;

use client_gateway::auth::{issue_hs256, Authenticator, DEV_HS256_SECRET};
use client_gateway::demo::{demo_seed, Demo};
use client_gateway::{ClientGatewayConfig, Hub};
use core_engine::api::CoreApi;
use core_engine::stack::{CoreStack, StackConfig};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: client-gateway [--demo] [--config client-gateway.toml] \
[--fix-config fix-gateway.toml] [--data-dir DIR] [--listen ADDR]";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let mut demo = false;
    let mut cfg_path = None;
    let mut fix_cfg = None;
    let mut listen = None;
    let mut data_dir = String::from("./data/core-engine");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--demo" => demo = true,
            "--config" => cfg_path = args.next(),
            "--fix-config" => fix_cfg = args.next(),
            "--listen" => listen = args.next(),
            "--data-dir" => {
                data_dir = args
                    .next()
                    .ok_or_else(|| format!("--data-dir needs a value\n{USAGE}"))?
            }
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
    cfg.apply_env()?;
    let auth = Authenticator::from_env()?;
    auth.enforce_dev_key_policy(demo)?;
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
        // fix-gateway + core-engine in-process against a configured LP; the
        // journal lives in --data-dir. Fresh journals get the demo seed (dev use).
        let gw_cfg = fix_gateway::GatewayConfig::load(p)?;
        let instruments = gw_cfg.instruments.clone();
        let mut st = StackConfig::new(gw_cfg.clone(), &data_dir);
        st.seed = Some(demo_seed(&cfg, &gw_cfg)?);
        let stack = CoreStack::start(st).await?;
        let core: Arc<dyn CoreApi> = stack.core.clone();
        let hub = Hub::with_instruments(cfg, auth, &instruments, Some(core.clone()));
        tokio::spawn(hub.clone().run_core_bridge(core.subscribe()));
        fix_handle = Some(stack);
        hub
    } else {
        tracing::warn!("no --demo / --fix-config: serving without a market data feed");
        Hub::new(cfg, auth, Vec::new(), None)
    };

    tracing::info!(%addr, "client-gateway listening (ws://{addr}/ws, /healthz, /metrics)");
    // Machine-readable line for scripts/tests (e.g. with `--listen 127.0.0.1:0`).
    println!("FXVPS_WS_URL=ws://{addr}/ws");
    if demo && dev_key {
        // Dev key only: a convenience token for local clients.
        let t = issue_hs256(
            DEV_HS256_SECRET.as_bytes(),
            "demo-user",
            &["DEMO-1", "DEMO-H1"],
            24 * 3600,
        );
        tracing::info!("demo token (accounts DEMO-1, DEMO-H1, dev key): {t}");
        println!("FXVPS_DEMO_TOKEN={t}");
    }
    tokio::select! {
        r = client_gateway::serve(hub, listener) => r?,
        _ = tokio::signal::ctrl_c() => {}
    }
    if let Some(d) = demo_handle {
        d.shutdown().await;
    }
    if let Some(s) = fix_handle {
        s.shutdown().await;
    }
    Ok(())
}
