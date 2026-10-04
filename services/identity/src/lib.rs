//! fxvps identity service.
//!
//! * Users: email + password (argon2id), email verification, password reset.
//! * Second factor: TOTP (with replay protection) and one-time recovery codes.
//! * Passkeys (WebAuthn, `webauthn-rs`) as a phishing-resistant login.
//! * Sessions: rotating, revocable refresh tokens stored hashed; reuse of a rotated
//!   token revokes the whole token family. Web clients get the refresh token in an
//!   `HttpOnly; SameSite=Strict` cookie, native clients in the response body.
//! * Short-lived RS256 access tokens (`kid`, `/.well-known/jwks.json`, OIDC
//!   discovery) carrying `sub`, `accounts`, `roles`, `amr` — what client-gateway checks.
//! * Brute-force defence: per-IP rate limit and per-account lockout. Every auth
//!   event is written to the audit log.
//!
//! See `services/identity/README.md` for the HTTP API.

pub mod config;
pub mod crypto;
pub mod http;
pub mod keys;
pub mod mail;
pub mod pg;
pub mod store;

use std::num::NonZeroU32;
use std::sync::Arc;

use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use serde::{Deserialize, Serialize};
use webauthn_rs::{Webauthn, WebauthnBuilder};

pub use config::Config;
use crypto::Passwords;
use keys::KeyRing;
use mail::Mailer;
use store::{AuditEvent, Store};

/// Roles understood by the platform.
pub const ROLES: &[&str] = &["client", "admin", "dealer", "risk", "support", "readonly"];

/// Access token claims (RS256). client-gateway reads `sub`, `exp`, `accounts`,
/// and validates `iss` / `aud`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessClaims {
    pub iss: String,
    pub aud: String,
    pub sub: String,
    pub exp: u64,
    pub iat: u64,
    pub jti: String,
    /// Session (refresh token family) id.
    pub sid: String,
    pub email: String,
    /// Trading account ids owned by the subject.
    pub accounts: Vec<String>,
    pub roles: Vec<String>,
    /// Authentication methods (RFC 8176): `pwd`, `otp`, `mfa`, `hwk`.
    pub amr: Vec<String>,
}

pub struct App {
    pub cfg: Config,
    pub store: Arc<dyn Store>,
    pub keys: KeyRing,
    pub mailer: Arc<dyn Mailer>,
    pub webauthn: Webauthn,
    pub passwords: Passwords,
    pub ip_limiter: DefaultKeyedRateLimiter<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("webauthn: {0}")]
    Webauthn(String),
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl App {
    pub fn new(
        cfg: Config,
        store: Arc<dyn Store>,
        keys: KeyRing,
        mailer: Arc<dyn Mailer>,
    ) -> Result<Arc<Self>, InitError> {
        let origin =
            url::Url::parse(&cfg.rp_origin).map_err(|e| InitError::Webauthn(e.to_string()))?;
        let mut b = WebauthnBuilder::new(&cfg.rp_id, &origin)
            .map_err(|e| InitError::Webauthn(e.to_string()))?
            .rp_name("fxvps")
            .allow_any_port(true);
        for o in &cfg.allowed_origins {
            if let Ok(u) = url::Url::parse(o) {
                if u != origin {
                    b = b.append_allowed_origin(&u);
                }
            }
        }
        let webauthn = b.build().map_err(|e| InitError::Webauthn(e.to_string()))?;
        let rpm = NonZeroU32::new(cfg.ip_requests_per_minute.max(1)).unwrap_or(NonZeroU32::MIN);
        Ok(Arc::new(App {
            passwords: Passwords::new(cfg.argon2_m_kib, cfg.argon2_t),
            ip_limiter: RateLimiter::keyed(Quota::per_minute(rpm)),
            cfg,
            store,
            keys,
            mailer,
            webauthn,
        }))
    }

    /// Records an auth event (and logs it). Audit failures are logged, not fatal.
    pub async fn audit(&self, user_id: Option<&str>, event: &str, ip: Option<&str>, detail: &str) {
        tracing::info!(target: "audit", user_id, event, ip, detail);
        let e = AuditEvent {
            ts: now(),
            user_id: user_id.map(str::to_string),
            event: event.into(),
            ip: ip.map(str::to_string),
            detail: detail.into(),
        };
        if let Err(err) = self.store.audit(&e).await {
            tracing::error!(error = %err, event, "audit write failed");
        }
    }

    /// Verifies an access token issued by this service.
    pub fn verify_access(&self, token: &str) -> Option<AccessClaims> {
        let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        v.set_issuer(&[&self.cfg.issuer]);
        v.set_audience(&[&self.cfg.audience]);
        v.leeway = 5;
        self.keys.verify(token, &v)
    }
}
