use core_engine::admin::{self, auth::Authenticator, AdminConfig};
use core_engine::{spawn, Settings};

/// `core-engine verify|compact [--data-dir DIR]`: offline tools for backups
/// and deploys (JSON report on stdout; exit 1 when the state is not ok).
fn offline(cmd: &str, mut args: std::env::Args) -> Result<bool, Box<dyn std::error::Error>> {
    let mut dir = std::env::var("CORE_DATA_DIR").unwrap_or_else(|_| "./data/core-engine".into());
    while let Some(a) = args.next() {
        if a == "--data-dir" {
            dir = args.next().ok_or("--data-dir needs a value")?;
        }
    }
    let ok = match cmd {
        "verify" => {
            let r = core_engine::verify(&dir)?;
            println!("{}", serde_json::to_string(&r)?);
            r.ok
        }
        "compact" => {
            let r = core_engine::compact(&dir)?;
            println!("{}", serde_json::to_string(&r)?);
            true
        }
        _ => unreachable!(),
    };
    Ok(ok)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut argv = std::env::args();
    argv.next();
    if let Some(cmd) = argv.next() {
        match cmd.as_str() {
            "verify" | "compact" => {
                if !offline(&cmd, argv)? {
                    std::process::exit(1);
                }
                return Ok(());
            }
            "-h" | "--help" => {
                println!("usage: core-engine [verify|compact --data-dir DIR]  (no args: admin API server)");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
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
    if let Some(url) = std::env::var("CORE_LP_ADMIN_URL")
        .ok()
        .filter(|v| !v.is_empty())
    {
        let token = std::env::var("CORE_LP_ADMIN_TOKEN").map_err(|_| {
            "CORE_LP_ADMIN_URL needs CORE_LP_ADMIN_TOKEN (= the gateway's FIX_ADMIN_TOKEN)"
        })?;
        admin_cfg.lp_admin = Some(admin::LpAdmin { url, token });
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
