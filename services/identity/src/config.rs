//! Environment configuration. No secrets have defaults: without a signing key a
//! clearly labelled ephemeral DEVELOPMENT key is generated at startup.

use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    /// `IDENTITY_LISTEN` (default `127.0.0.1:8090`).
    pub listen: String,
    /// `IDENTITY_ISSUER`: `iss` claim and base URL in the discovery document.
    pub issuer: String,
    /// `IDENTITY_AUDIENCE`: `aud` claim (default `fxvps`).
    pub audience: String,
    /// `DATABASE_URL`; in-memory store when unset (development).
    pub database_url: Option<String>,
    /// `IDENTITY_SIGNING_KEY_FILE`: RSA private key PEM (PKCS#8 or PKCS#1).
    pub signing_key_file: Option<String>,
    /// `IDENTITY_RETIRED_KEY_FILES`: comma separated PEMs still published in the JWKS.
    pub retired_key_files: Vec<String>,
    /// `IDENTITY_RP_ID` (WebAuthn relying party id, default `localhost`).
    pub rp_id: String,
    /// `IDENTITY_RP_ORIGIN` (default `http://localhost:5173`).
    pub rp_origin: String,
    /// `IDENTITY_ALLOWED_ORIGINS`: CORS + cookie-mode origin allow list (comma separated).
    pub allowed_origins: Vec<String>,
    /// `IDENTITY_PUBLIC_URL`: base for links in emails (default: `rp_origin`).
    pub public_url: String,
    /// `IDENTITY_BOOTSTRAP_ADMINS`: emails that receive the `admin` role on registration.
    pub bootstrap_admins: Vec<String>,
    /// `IDENTITY_SERVICE_TOKEN`: static bearer accepted on `/v1/admin/*` (service-to-service).
    pub service_token: Option<String>,
    /// `IDENTITY_COOKIE_SECURE` (default true).
    pub cookie_secure: bool,
    /// `IDENTITY_TRUST_PROXY`: take the client IP from `X-Forwarded-For`.
    pub trust_proxy: bool,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    pub verify_ttl: Duration,
    pub reset_ttl: Duration,
    pub mfa_ttl: Duration,
    /// Failed logins before lockout.
    pub max_failed_logins: i32,
    pub lockout: Duration,
    /// Auth requests per minute per client IP.
    pub ip_requests_per_minute: u32,
    /// argon2id memory cost (KiB) / iterations. Lowered in tests only.
    pub argon2_m_kib: u32,
    pub argon2_t: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen: "127.0.0.1:8090".into(),
            issuer: "http://127.0.0.1:8090".into(),
            audience: "fxvps".into(),
            database_url: None,
            signing_key_file: None,
            retired_key_files: Vec::new(),
            rp_id: "localhost".into(),
            rp_origin: "http://localhost:5173".into(),
            allowed_origins: vec![
                "http://localhost:5173".into(),
                "http://127.0.0.1:5173".into(),
            ],
            public_url: "http://localhost:5173".into(),
            bootstrap_admins: Vec::new(),
            service_token: None,
            cookie_secure: true,
            trust_proxy: false,
            access_ttl: Duration::from_secs(300),
            refresh_ttl: Duration::from_secs(30 * 24 * 3600),
            verify_ttl: Duration::from_secs(24 * 3600),
            reset_ttl: Duration::from_secs(3600),
            mfa_ttl: Duration::from_secs(300),
            max_failed_logins: 5,
            lockout: Duration::from_secs(15 * 60),
            ip_requests_per_minute: 60,
            argon2_m_kib: 19 * 1024,
            argon2_t: 2,
        }
    }
}

fn list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let mut c = Config::default();
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let secs = |k: &str, d: Duration| -> Result<Duration, String> {
            match var(k) {
                Some(v) => v
                    .parse::<u64>()
                    .map(Duration::from_secs)
                    .map_err(|e| format!("{k}: {e}")),
                None => Ok(d),
            }
        };
        if let Some(v) = var("IDENTITY_LISTEN") {
            c.listen = v;
        }
        if let Some(v) = var("IDENTITY_ISSUER") {
            c.issuer = v.trim_end_matches('/').to_string();
        }
        if let Some(v) = var("IDENTITY_AUDIENCE") {
            c.audience = v;
        }
        c.database_url = var("DATABASE_URL");
        c.signing_key_file = var("IDENTITY_SIGNING_KEY_FILE");
        c.retired_key_files = var("IDENTITY_RETIRED_KEY_FILES")
            .map(|v| list(&v))
            .unwrap_or_default();
        if let Some(v) = var("IDENTITY_RP_ID") {
            c.rp_id = v;
        }
        if let Some(v) = var("IDENTITY_RP_ORIGIN") {
            c.rp_origin = v.trim_end_matches('/').to_string();
            c.public_url = c.rp_origin.clone();
        }
        if let Some(v) = var("IDENTITY_ALLOWED_ORIGINS") {
            c.allowed_origins = list(&v);
        }
        if let Some(v) = var("IDENTITY_PUBLIC_URL") {
            c.public_url = v.trim_end_matches('/').to_string();
        }
        if let Some(v) = var("IDENTITY_BOOTSTRAP_ADMINS") {
            c.bootstrap_admins = list(&v.to_lowercase());
        }
        c.service_token = var("IDENTITY_SERVICE_TOKEN");
        if let Some(v) = var("IDENTITY_COOKIE_SECURE") {
            c.cookie_secure = !matches!(v.as_str(), "0" | "false" | "no");
        }
        if let Some(v) = var("IDENTITY_TRUST_PROXY") {
            c.trust_proxy = matches!(v.as_str(), "1" | "true" | "yes");
        }
        c.access_ttl = secs("IDENTITY_ACCESS_TTL_SECS", c.access_ttl)?;
        c.refresh_ttl = secs("IDENTITY_REFRESH_TTL_SECS", c.refresh_ttl)?;
        if let Some(v) = var("IDENTITY_IP_REQUESTS_PER_MINUTE") {
            c.ip_requests_per_minute = v
                .parse()
                .map_err(|e| format!("IDENTITY_IP_REQUESTS_PER_MINUTE: {e}"))?;
        }
        if c.service_token.as_ref().is_some_and(|t| t.len() < 32) {
            return Err("IDENTITY_SERVICE_TOKEN must be at least 32 characters".into());
        }
        Ok(c)
    }
}
