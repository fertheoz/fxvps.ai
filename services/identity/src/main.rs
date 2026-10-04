use std::net::SocketAddr;
use std::sync::Arc;

use identity::keys::{parse_private_pem, KeyRing};
use identity::mail::LogMailer;
use identity::pg::PgStore;
use identity::store::{MemoryStore, Store};
use identity::{App, Config};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if std::env::args().any(|a| a == "-h" || a == "--help") {
        println!("identity: configured by environment (see services/identity/README.md)");
        return Ok(());
    }
    let cfg = Config::from_env()?;

    let store: Arc<dyn Store> = match &cfg.database_url {
        Some(url) => Arc::new(PgStore::connect(url).await?),
        None => {
            tracing::warn!(
                "DATABASE_URL not set: DEVELOPMENT in-memory store, all data is lost on exit"
            );
            Arc::new(MemoryStore::new())
        }
    };

    let keys = match &cfg.signing_key_file {
        Some(p) => {
            let active = parse_private_pem(&std::fs::read_to_string(p)?)?;
            let mut retired = Vec::new();
            for f in &cfg.retired_key_files {
                retired.push(parse_private_pem(&std::fs::read_to_string(f)?)?);
            }
            KeyRing::new(&active, &retired, false)?
        }
        None => {
            tracing::warn!(
                "IDENTITY_SIGNING_KEY_FILE not set: generated an EPHEMERAL DEVELOPMENT signing key; \
                 tokens become invalid on restart. Never run like this in production."
            );
            KeyRing::ephemeral()?
        }
    };
    tracing::info!(kid = %keys.active_kid(), issuer = %cfg.issuer, "signing key loaded");

    // TODO(prod): plug an SMTP / provider mailer; LogMailer only logs.
    tracing::warn!("using the DEVELOPMENT log mailer: emails (verification/reset links) are written to the log");
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    let addr = listener.local_addr()?;
    let mailer = LogMailer {
        file: std::env::var_os("IDENTITY_DEV_MAIL_FILE").map(Into::into),
    };
    let app = App::new(cfg, store, keys, Arc::new(mailer))?;
    tracing::info!(%addr, "identity listening");
    // Machine-readable line for scripts/tests (e.g. `IDENTITY_LISTEN=127.0.0.1:0`).
    println!("FXVPS_IDENTITY_URL=http://{addr}");
    let svc = identity::http::router(app).into_make_service_with_connect_info::<SocketAddr>();
    tokio::select! {
        r = axum::serve(listener, svc) => r?,
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
}
