use std::net::SocketAddr;
use std::sync::Arc;

use identity::keys::{parse_private_pem, KeyRing};
use identity::mail::{select_mailer, LogMailer, Mailer, MailerKind, SmtpMailer};
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
    let args: Vec<String> = std::env::args().collect();
    // Operator commands against the production store (DATABASE_URL).
    let op = args.get(1).map(String::as_str);
    if matches!(op, Some("verify-email" | "link-account")) {
        let url =
            std::env::var("DATABASE_URL").map_err(|_| "operator commands need DATABASE_URL")?;
        let store = PgStore::connect(&url).await?;
        let email = args.get(2).ok_or(
            "usage: identity verify-email <email> | identity link-account <email> <account>",
        )?;
        if op == Some("verify-email") {
            let changed = identity::store::verify_email(&store, email).await?;
            println!(
                "{}",
                if changed {
                    "email verified"
                } else {
                    "already verified"
                }
            );
        } else {
            let account = args
                .get(3)
                .ok_or("usage: identity link-account <email> <account>")?;
            identity::store::link_account(&store, email, account).await?;
            println!("account {account} linked");
        }
        return Ok(());
    }
    if args.get(1).map(String::as_str) == Some("grant-admin") {
        let email = args.get(2).ok_or("usage: identity grant-admin <email>")?;
        let url = std::env::var("DATABASE_URL").map_err(|_| "grant-admin needs DATABASE_URL")?;
        let store = PgStore::connect(&url).await?;
        let changed = identity::store::grant_admin(&store, email).await?;
        println!(
            "{}",
            if changed {
                "admin granted"
            } else {
                "already admin"
            }
        );
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

    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let smtp_url = var("IDENTITY_SMTP_URL");
    let dev_mailer = var("IDENTITY_DEV_MAILER").is_some_and(|v| v == "1" || v == "true");
    let mailer: Arc<dyn Mailer> =
        match select_mailer(smtp_url.as_deref(), cfg.database_url.is_some(), dev_mailer)? {
            MailerKind::Smtp => {
                let from = var("IDENTITY_MAIL_FROM")
                    .ok_or("IDENTITY_MAIL_FROM is required with IDENTITY_SMTP_URL")?;
                Arc::new(SmtpMailer::from_url(
                    smtp_url.as_deref().unwrap_or_default(),
                    &from,
                )?)
            }
            MailerKind::Log => {
                tracing::warn!("using the DEVELOPMENT log mailer: emails are not sent");
                Arc::new(LogMailer {
                    file: std::env::var_os("IDENTITY_DEV_MAIL_FILE").map(Into::into),
                })
            }
        };
    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    let addr = listener.local_addr()?;
    let app = App::new(cfg, store, keys, mailer)?;
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
