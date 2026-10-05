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

    // Managed LP config (written by the back office through the admin API).
    let managed_path = std::env::var("FIX_CONFIG_FILE")
        .ok()
        .filter(|v| !v.is_empty());
    let fix_cfg = fix_cfg.or_else(|| managed_path.clone());
    let gw_cfg = match &fix_cfg {
        None => None,
        Some(p) if p.ends_with(".json") => match std::fs::read(p) {
            Ok(b) => Some(serde_json::from_slice::<fix_gateway::GatewayConfig>(&b)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %p, "no LP configured yet: trading disabled until the console saves one");
                None
            }
            Err(e) => return Err(e.into()),
        },
        Some(p) => Some(fix_gateway::GatewayConfig::load(p)?),
    };
    // Paused from the console (Stop): no LP connection at all until Play.
    let gw_cfg = gw_cfg.filter(|c| {
        if !c.enabled {
            tracing::warn!("LP connection paused (enabled=false): no logon attempts");
        }
        c.enabled
    });

    let mut demo_handle = None;
    let mut fix_handle = None;
    let mut admin_engine = None;
    let mut lp_status = None;
    let hub: Arc<Hub> = if demo {
        let d = Demo::start(cfg, auth, None).await?;
        let hub = d.hub.clone();
        demo_handle = Some(d);
        hub
    } else if let Some(gw_cfg) = gw_cfg {
        // fix-gateway + core-engine in-process against a configured LP; the
        // journal lives in --data-dir. Fresh journals get the demo seed (dev use).
        let instruments = gw_cfg.instruments.clone();
        let mut st = StackConfig::new(gw_cfg.clone(), &data_dir);
        st.seed = Some(demo_seed(&cfg, &gw_cfg)?);
        let stack = CoreStack::start(st).await?;
        let core: Arc<dyn CoreApi> = stack.core.clone();
        let hub = Hub::with_instruments(cfg, auth, &instruments, Some(core.clone()));
        tokio::spawn(hub.clone().run_core_bridge(core.subscribe()));
        // Chart history survives restarts: the LP gives no historical candles.
        // `candles-seed.txt` (optional back-fill from another source, see
        // deploy/lmax-demo/seed-candles.py) is loaded first: our own bars win.
        let candle_file = std::path::Path::new(&data_dir).join("candles.txt");
        for f in [
            candle_file.with_file_name("candles-seed.txt"),
            candle_file.clone(),
        ] {
            match hub.load_candles(&f) {
                Ok(n) => tracing::info!(bars = n, file = %f.display(), "candle history restored"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::warn!("candle history not restored: {e}"),
            }
        }
        let saver = hub.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(300));
            tick.tick().await;
            loop {
                tick.tick().await;
                let (h, f) = (saver.clone(), candle_file.clone());
                match tokio::task::spawn_blocking(move || h.save_candles(&f)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => tracing::warn!("candle history not saved: {e}"),
                    Err(e) => tracing::warn!("candle history not saved: {e}"),
                }
            }
        });
        admin_engine = Some(stack.engine.clone());
        lp_status = Some(stack.lp_status());
        fix_handle = Some(stack);
        hub
    } else {
        tracing::warn!("no --demo / --fix-config: serving without a market data feed");
        Hub::new(cfg, auth, Vec::new(), None)
    };

    // Client preferences (chart objects, alerts) live next to the engine data.
    let prefs_file = std::path::Path::new(&data_dir).join("prefs.json");
    match hub.use_prefs_file(prefs_file) {
        Ok(n) => tracing::info!(accounts = n, "client preferences loaded"),
        Err(e) => tracing::warn!("client preferences not loaded: {e}"),
    }
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
    // Back office admin API on the same engine (`CORE_ADMIN_ADDR`).
    let restart = Arc::new(tokio::sync::Notify::new());
    let mut standalone = None;
    if let Some(addr) = std::env::var("CORE_ADMIN_ADDR")
        .ok()
        .filter(|v| !v.is_empty())
    {
        use core_engine::admin;
        let flag = |k: &str| std::env::var(k).is_ok_and(|v| v == "1" || v == "true");
        let engine = match admin_engine.take() {
            Some(e) => e,
            None => {
                // No LP yet: an engine without LP routing so the console works.
                let (h, join) = core_engine::spawn(core_engine::Settings::new(&data_dir))?;
                standalone = Some((h.clone(), join));
                h
            }
        };
        let auth =
            admin::auth::Authenticator::from_env(flag("CORE_DEV_AUTH"), &addr).map_err(|e| e.0)?;
        let mut acfg = admin::AdminConfig::new(&data_dir).with_env(flag("CORE_DEV_AUTH"))?;
        let table = lp_status.clone().unwrap_or_default();
        acfg.lp_status = Some(table.clone());
        if let Some(path) = &managed_path {
            let token = std::env::var("FIX_ADMIN_TOKEN")
                .ok()
                .filter(|t| t.len() >= 16)
                .ok_or("FIX_CONFIG_FILE needs FIX_ADMIN_TOKEN (at least 16 characters)")?;
            let managed = Arc::new(fix_gateway::managed::Managed::open(path)?);
            let status_addr =
                std::env::var("FIX_STATUS_ADDR").unwrap_or_else(|_| "127.0.0.1:9890".into());
            let local = fix_gateway::status_http::spawn_with_admin(
                &status_addr,
                table,
                Some(fix_gateway::status_http::Admin {
                    token: token.clone(),
                    managed: managed.clone(),
                }),
            )
            .await?;
            acfg.lp_admin = Some(admin::LpAdmin {
                url: format!("http://{local}"),
                token,
            });
            let restart = restart.clone();
            tokio::spawn(async move {
                managed.changed().await;
                tracing::info!("LP config changed: restarting to apply it");
                restart.notify_one();
            });
        }
        let app = admin::app(engine, auth, acfg)?;
        let l = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(%addr, "back office admin API");
        tokio::spawn(async move {
            if let Err(e) = axum::serve(l, app).await {
                tracing::error!(error = %e, "admin API stopped");
            }
        });
    }

    tokio::select! {
        r = client_gateway::serve(hub, listener) => r?,
        _ = tokio::signal::ctrl_c() => {}
        _ = restart.notified() => {}
    }
    if let Some((h, join)) = standalone {
        h.shutdown();
        let _ = join.join();
    }
    if let Some(d) = demo_handle {
        d.shutdown().await;
    }
    if let Some(s) = fix_handle {
        s.shutdown().await;
    }
    Ok(())
}
