//! JWT bearer authentication.
//!
//! Key sources, in order of precedence (see [`Authenticator::from_env`]):
//! 1. `FXVPS_JWT_JWKS_URL` — JWKS fetched over HTTP(S) from the identity service
//!    (`/.well-known/jwks.json`), cached and refreshed every
//!    `FXVPS_JWT_JWKS_REFRESH_SECS` (default 300) and on an unknown `kid`
//!    (key rotation), at most once per `FXVPS_JWT_JWKS_MIN_REFRESH_SECS` (default 10),
//! 2. `FXVPS_JWT_JWKS_FILE` — JWKS JSON (RS256 / ES256 keys selected by `kid`),
//! 3. `FXVPS_JWT_RS256_PUBLIC_KEY_FILE` — PEM public key,
//! 4. `FXVPS_JWT_HS256_SECRET` — shared secret,
//! 5. otherwise the **development-only** [`DEV_HS256_SECRET`]. The binary refuses
//!    to start with it unless `--demo` or `FXVPS_DEV_AUTH=1` is given
//!    (see [`Authenticator::enforce_dev_key_policy`]).
//!
//! `FXVPS_JWT_ISSUER` / `FXVPS_JWT_AUDIENCE`, when set, make `iss` / `aud`
//! mandatory and checked (always set them with the identity service).

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header};
use serde::{Deserialize, Serialize};

/// DEVELOPMENT ONLY. Public, insecure default so `--demo` works out of the box.
/// Never accepted as a production secret: set `FXVPS_JWT_HS256_SECRET` or a JWKS.
pub const DEV_HS256_SECRET: &str = "fxvps-DEV-ONLY-insecure-hs256-key-not-for-production";

/// Token claims. `accounts` lists the trading accounts the subject may read and trade.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: u64,
    #[serde(default)]
    pub accounts: Vec<String>,
    /// Platform roles (identity service): client, admin, dealer, risk, support, readonly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Authentication methods (RFC 8176), e.g. `pwd`, `otp`, `mfa`, `hwk`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub amr: Vec<String>,
    /// API-key tokens: `read` (no trading) or `trade`; `None` = interactive login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl Claims {
    /// Read-only API keys may subscribe and query but never trade.
    pub fn may_trade(&self) -> bool {
        self.scope.as_deref() != Some("read")
    }
}

impl Claims {
    pub fn may_access(&self, account_id: &str) -> bool {
        self.accounts.iter().any(|a| a == account_id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid token: {0}")]
    Invalid(#[from] jsonwebtoken::errors::Error),
    #[error("no key for token (alg {alg:?}, kid {kid:?})")]
    NoKey { alg: Algorithm, kid: Option<String> },
    #[error("key config: {0}")]
    Config(String),
    #[error("jwks fetch: {0}")]
    Fetch(String),
}

struct Key {
    kid: Option<String>,
    alg: Algorithm,
    key: DecodingKey,
}

fn jwks_keys(set: &JwkSet) -> Result<Vec<Key>, AuthError> {
    let mut keys = Vec::new();
    for jwk in &set.keys {
        let alg = match jwk.common.key_algorithm {
            Some(a) => a
                .to_string()
                .parse::<Algorithm>()
                .map_err(|e| AuthError::Config(e.to_string()))?,
            None => Algorithm::RS256,
        };
        keys.push(Key {
            kid: jwk.common.key_id.clone(),
            alg,
            key: DecodingKey::from_jwk(jwk)?,
        });
    }
    Ok(keys)
}

/// Remote JWKS endpoint with refresh bookkeeping.
struct Remote {
    url: String,
    client: reqwest::Client,
    min_interval: Duration,
    last_attempt: Mutex<Option<Instant>>,
}

struct Inner {
    keys: RwLock<Vec<Key>>,
    issuer: Option<String>,
    audience: Option<String>,
    remote: Option<Remote>,
}

/// Cheap to clone; clones share the key cache.
#[derive(Clone)]
pub struct Authenticator {
    inner: Arc<Inner>,
    /// True when running with [`DEV_HS256_SECRET`].
    pub dev_key: bool,
}

impl Authenticator {
    fn with_keys(keys: Vec<Key>, dev_key: bool) -> Self {
        Authenticator {
            inner: Arc::new(Inner {
                keys: RwLock::new(keys),
                issuer: None,
                audience: None,
                remote: None,
            }),
            dev_key,
        }
    }

    pub fn hs256(secret: &[u8]) -> Self {
        Self::with_keys(
            vec![Key {
                kid: None,
                alg: Algorithm::HS256,
                key: DecodingKey::from_secret(secret),
            }],
            secret == DEV_HS256_SECRET.as_bytes(),
        )
    }

    pub fn rs256_pem(pem: &[u8]) -> Result<Self, AuthError> {
        Ok(Self::with_keys(
            vec![Key {
                kid: None,
                alg: Algorithm::RS256,
                key: DecodingKey::from_rsa_pem(pem)?,
            }],
            false,
        ))
    }

    /// Keys from a JWKS document.
    pub fn jwks(set: &JwkSet) -> Result<Self, AuthError> {
        let keys = jwks_keys(set)?;
        if keys.is_empty() {
            return Err(AuthError::Config("empty JWKS".into()));
        }
        Ok(Self::with_keys(keys, false))
    }

    /// Keys fetched from `url` (the identity service's `jwks_uri`). Starts empty;
    /// call [`Authenticator::refresh`] (or [`Authenticator::spawn_refresh`]).
    pub fn jwks_url(url: &str, min_refresh: Duration) -> Result<Self, AuthError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AuthError::Config(e.to_string()))?;
        Ok(Authenticator {
            inner: Arc::new(Inner {
                keys: RwLock::new(Vec::new()),
                issuer: None,
                audience: None,
                remote: Some(Remote {
                    url: url.to_string(),
                    client,
                    min_interval: min_refresh,
                    last_attempt: Mutex::new(None),
                }),
            }),
            dev_key: false,
        })
    }

    /// Requires and checks `iss` / `aud`. Call before cloning / sharing.
    pub fn with_validation(mut self, issuer: Option<String>, audience: Option<String>) -> Self {
        if let Some(inner) = Arc::get_mut(&mut self.inner) {
            inner.issuer = issuer;
            inner.audience = audience;
        }
        self
    }

    pub fn from_env() -> Result<Self, AuthError> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let secs = |k: &str, d: u64| var(k).and_then(|v| v.parse().ok()).unwrap_or(d);
        let read =
            |p: String| std::fs::read(&p).map_err(|e| AuthError::Config(format!("{p}: {e}")));
        let auth = if let Some(url) = var("FXVPS_JWT_JWKS_URL") {
            Self::jwks_url(
                &url,
                Duration::from_secs(secs("FXVPS_JWT_JWKS_MIN_REFRESH_SECS", 10)),
            )?
        } else if let Some(p) = var("FXVPS_JWT_JWKS_FILE") {
            let set: JwkSet =
                serde_json::from_slice(&read(p)?).map_err(|e| AuthError::Config(e.to_string()))?;
            Self::jwks(&set)?
        } else if let Some(p) = var("FXVPS_JWT_RS256_PUBLIC_KEY_FILE") {
            Self::rs256_pem(&read(p)?)?
        } else if let Some(s) = var("FXVPS_JWT_HS256_SECRET") {
            Self::hs256(s.as_bytes())
        } else {
            Self::hs256(DEV_HS256_SECRET.as_bytes())
        };
        let auth = auth.with_validation(var("FXVPS_JWT_ISSUER"), var("FXVPS_JWT_AUDIENCE"));
        if auth.inner.remote.is_some() {
            auth.spawn_refresh(Duration::from_secs(secs(
                "FXVPS_JWT_JWKS_REFRESH_SECS",
                300,
            )));
        }
        Ok(auth)
    }

    /// Fetches the remote JWKS now and replaces the cached keys. Keys absent
    /// from the new document stop being accepted (retired by the issuer).
    /// No-op for static key sources.
    pub async fn refresh(&self) -> Result<usize, AuthError> {
        let Some(r) = &self.inner.remote else {
            return Ok(self.read_keys().len());
        };
        *r.last_attempt.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        let resp = r
            .client
            .get(&r.url)
            .send()
            .await
            .and_then(|x| x.error_for_status())
            .map_err(|e| AuthError::Fetch(e.to_string()))?;
        let set: JwkSet = resp
            .json()
            .await
            .map_err(|e| AuthError::Fetch(e.to_string()))?;
        let keys = jwks_keys(&set)?;
        if keys.is_empty() {
            return Err(AuthError::Fetch("empty JWKS".into()));
        }
        let n = keys.len();
        *self.inner.keys.write().unwrap_or_else(|e| e.into_inner()) = keys;
        tracing::debug!(url = %r.url, keys = n, "JWKS refreshed");
        Ok(n)
    }

    /// Background refresh: immediately, then every `every`. Needs a Tokio runtime
    /// (no-op without one or for static keys).
    pub fn spawn_refresh(&self, every: Duration) {
        if self.inner.remote.is_none() {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let weak = Arc::downgrade(&self.inner);
        let dev_key = self.dev_key;
        rt.spawn(async move {
            loop {
                let Some(inner) = weak.upgrade() else { return };
                let a = Authenticator { inner, dev_key };
                let ok = match a.refresh().await {
                    Ok(_) => true,
                    Err(e) => {
                        tracing::warn!(error = %e, "JWKS refresh failed; keeping cached keys");
                        false
                    }
                };
                drop(a);
                // Retry sooner while we have nothing / the fetch failed.
                let wait = if ok {
                    every
                } else {
                    every.min(Duration::from_secs(5))
                };
                tokio::time::sleep(wait).await;
            }
        });
    }

    /// Unknown `kid` (likely a rotation): refresh in the background, rate limited.
    fn poke_refresh(&self) {
        let Some(r) = &self.inner.remote else { return };
        {
            let mut last = r.last_attempt.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|t| t.elapsed() < r.min_interval) {
                return;
            }
            *last = Some(Instant::now());
        }
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let a = self.clone();
            rt.spawn(async move {
                if let Err(e) = a.refresh().await {
                    tracing::warn!(error = %e, "JWKS refresh (unknown kid) failed");
                }
            });
        }
    }

    fn read_keys(&self) -> std::sync::RwLockReadGuard<'_, Vec<Key>> {
        self.inner.keys.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn verify(&self, token: &str) -> Result<Claims, AuthError> {
        let header = decode_header(token)?;
        let keys = self.read_keys();
        let Some(key) = keys
            .iter()
            .find(|k| k.alg == header.alg && (header.kid.is_none() || k.kid == header.kid))
        else {
            drop(keys);
            self.poke_refresh();
            return Err(AuthError::NoKey {
                alg: header.alg,
                kid: header.kid.clone(),
            });
        };
        let mut v = jsonwebtoken::Validation::new(key.alg);
        v.leeway = 5;
        let mut req = vec!["exp", "sub"];
        if let Some(i) = &self.inner.issuer {
            v.set_issuer(&[i]);
            req.push("iss");
        }
        match &self.inner.audience {
            Some(a) => {
                v.set_audience(&[a]);
                req.push("aud");
            }
            None => v.validate_aud = false,
        }
        v.set_required_spec_claims(&req);
        Ok(decode::<Claims>(token, &key.key, &v)?.claims)
    }
}

/// Startup policy for the built-in development key (finding G1): a gateway
/// without a configured key source may only start in `--demo` mode or with an
/// explicit `FXVPS_DEV_AUTH=1`; otherwise anyone could forge tokens for any
/// account with the public [`DEV_HS256_SECRET`].
pub fn dev_key_allowed(dev_key: bool, demo: bool, dev_auth_flag: Option<&str>) -> bool {
    !dev_key || demo || dev_auth_flag == Some("1")
}

impl Authenticator {
    /// Fails when the development key is in use without `--demo` or
    /// `FXVPS_DEV_AUTH=1`.
    pub fn enforce_dev_key_policy(&self, demo: bool) -> Result<(), AuthError> {
        let flag = std::env::var("FXVPS_DEV_AUTH").ok();
        if dev_key_allowed(self.dev_key, demo, flag.as_deref()) {
            return Ok(());
        }
        Err(AuthError::Config(
            "no JWT key configured: set FXVPS_JWT_JWKS_URL (identity), FXVPS_JWT_JWKS_FILE, \
             FXVPS_JWT_RS256_PUBLIC_KEY_FILE or FXVPS_JWT_HS256_SECRET; the built-in \
             development key is only allowed with --demo or FXVPS_DEV_AUTH=1"
                .into(),
        ))
    }
}

/// Issues an HS256 token (tests and `--demo` only).
pub fn issue_hs256(secret: &[u8], sub: &str, accounts: &[&str], ttl_secs: u64) -> String {
    let claims = Claims {
        sub: sub.into(),
        exp: domain::now_ns() / 1_000_000_000 + ttl_secs,
        accounts: accounts.iter().map(|s| s.to_string()).collect(),
        roles: Vec::new(),
        amr: Vec::new(),
        scope: None,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_rejects_bad() {
        let a = Authenticator::hs256(b"k1");
        let t = issue_hs256(b"k1", "u", &["A1"], 60);
        let c = a.verify(&t).unwrap();
        assert_eq!(c.sub, "u");
        assert!(c.may_access("A1") && !c.may_access("A2"));
        // wrong key
        assert!(a.verify(&issue_hs256(b"other", "u", &["A1"], 60)).is_err());
        // garbage
        assert!(a.verify("not.a.jwt").is_err());
        // expired (beyond leeway)
        let expired = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                sub: "u".into(),
                exp: 1_000,
                accounts: vec![],
                roles: vec![],
                amr: vec![],
                scope: None,
            },
            &EncodingKey::from_secret(b"k1"),
        )
        .unwrap();
        assert!(a.verify(&expired).is_err());
        // alg mismatch: HS384 token against HS256 key
        let hs384 = encode(
            &Header::new(Algorithm::HS384),
            &Claims {
                sub: "u".into(),
                exp: u64::MAX / 2,
                accounts: vec![],
                roles: vec![],
                amr: vec![],
                scope: None,
            },
            &EncodingKey::from_secret(b"k1"),
        )
        .unwrap();
        assert!(matches!(a.verify(&hs384), Err(AuthError::NoKey { .. })));
    }

    #[test]
    fn read_scope_cannot_trade() {
        let a = Authenticator::hs256(b"k1");
        let t = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                sub: "u".into(),
                exp: u64::MAX / 2,
                accounts: vec!["A1".into()],
                roles: vec!["client".into()],
                amr: vec!["apikey".into()],
                scope: Some("read".into()),
            },
            &EncodingKey::from_secret(b"k1"),
        )
        .unwrap();
        let c = a.verify(&t).unwrap();
        assert!(c.may_access("A1") && !c.may_trade());
        assert!(a
            .verify(&issue_hs256(b"k1", "u", &["A1"], 60))
            .unwrap()
            .may_trade());
    }

    #[test]
    fn dev_key_is_flagged() {
        assert!(Authenticator::hs256(DEV_HS256_SECRET.as_bytes()).dev_key);
        assert!(!Authenticator::hs256(b"x").dev_key);
    }

    #[test]
    fn dev_key_needs_demo_or_explicit_flag() {
        // Configured key: always fine.
        assert!(dev_key_allowed(false, false, None));
        // Dev key: refused by default, allowed with --demo or FXVPS_DEV_AUTH=1 only.
        assert!(!dev_key_allowed(true, false, None));
        assert!(!dev_key_allowed(true, false, Some("0")));
        assert!(!dev_key_allowed(true, false, Some("true")));
        assert!(dev_key_allowed(true, true, None));
        assert!(dev_key_allowed(true, false, Some("1")));
        assert!(Authenticator::hs256(b"x")
            .enforce_dev_key_policy(false)
            .is_ok());
        assert!(Authenticator::hs256(DEV_HS256_SECRET.as_bytes())
            .enforce_dev_key_policy(true)
            .is_ok());
    }

    #[test]
    fn iss_aud_enforced_when_configured() {
        #[derive(Serialize)]
        struct C<'a> {
            sub: &'a str,
            exp: u64,
            iss: &'a str,
            aud: &'a str,
        }
        let tok = |iss, aud| {
            encode(
                &Header::new(Algorithm::HS256),
                &C {
                    sub: "u",
                    exp: u64::MAX / 2,
                    iss,
                    aud,
                },
                &EncodingKey::from_secret(b"k"),
            )
            .unwrap()
        };
        let a = Authenticator::hs256(b"k")
            .with_validation(Some("https://id".into()), Some("fxvps".into()));
        assert!(a.verify(&tok("https://id", "fxvps")).is_ok());
        assert!(a.verify(&tok("https://evil", "fxvps")).is_err());
        assert!(a.verify(&tok("https://id", "other")).is_err());
        // missing iss/aud entirely
        assert!(a.verify(&issue_hs256(b"k", "u", &[], 60)).is_err());
        // unconfigured: aud ignored
        assert!(Authenticator::hs256(b"k").verify(&tok("x", "y")).is_ok());
    }
}
