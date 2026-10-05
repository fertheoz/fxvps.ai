//! JWT bearer authentication and the role/permission matrix of the back
//! office (mirrors `apps/backoffice/src/lib/rbac.ts`; the server is the
//! authority, the UI guards are UX only).
//!
//! Key sources (see [`Authenticator::from_env`]), production first:
//! - `CORE_JWT_JWKS_URL`: the identity service JWKS (`/.well-known/jwks.json`,
//!   RS256 keys selected by `kid`), refreshed every `CORE_JWT_JWKS_REFRESH_SECS`
//!   (default 300). **Primary production path.**
//! - `CORE_JWT_JWKS_FILE`: the same JWKS as a file,
//! - `CORE_JWT_RS256_PUBLIC_KEY_FILE`: PEM public key (RS256),
//! - `CORE_JWT_HS256_SECRET`: shared secret (HS256, legacy / tests).
//!
//! `CORE_JWT_ISSUER` / `CORE_JWT_AUDIENCE` make `iss` / `aud` mandatory and
//! checked (set them to the identity service values). Leeway is 5 s.
//! `CORE_REQUIRE_MFA=1` requires an MFA method in `amr` (`mfa`, `otp`, `hwk`)
//! for every mutating permission (anything not `*.view`).
//!
//! Development (`CORE_DEV_AUTH=1`, see [`dev_auth_policy`]): `POST /auth/dev-token`
//! signs with a **random per-process key** (never a configured secret), is
//! refused together with any `CORE_JWT_*` key, only on a loopback bind, and mints
//! `admin` only with `CORE_DEV_AUTH_ADMIN=1`. The token `sub` is derived by the
//! server from the role (`dev-<role>`), never chosen by the caller.
//!
//! Four-eyes approvals compare the token `sub`. In production that is the
//! identity service user id (server-side identity, signature + `iss` checked),
//! which a caller cannot choose; dev tokens are only distinct per role.

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{
    decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// `iss` of dev tokens (validated separately from production keys).
pub const DEV_ISSUER: &str = "core-engine-dev";
/// Clock skew allowed on `exp` (seconds).
pub const LEEWAY_SECS: u64 = 5;
/// `amr` values that count as multi-factor (RFC 8176).
pub const MFA_AMR: [&str; 4] = ["mfa", "otp", "hwk", "swk"];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Dealer,
    Risk,
    Support,
    Readonly,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::Admin,
        Role::Dealer,
        Role::Risk,
        Role::Support,
        Role::Readonly,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Dealer => "dealer",
            Role::Risk => "risk",
            Role::Support => "support",
            Role::Readonly => "readonly",
        }
    }
    pub fn parse(s: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

pub const PERMISSIONS: [&str; 25] = [
    "dashboard.view",
    "clients.view",
    "clients.edit",
    "balance.deposit",
    "balance.withdraw",
    "balance.credit",
    "balance.approve",
    "groups.view",
    "groups.edit",
    "symbols.view",
    "symbols.edit",
    "positions.view",
    "positions.forceClose",
    "risk.view",
    "risk.edit",
    "lp.view",
    "lp.reconnect",
    "lp.manage",
    "reports.view",
    "reports.export",
    "audit.view",
    "users.view",
    "users.edit",
    "settings.view",
    "settings.edit",
];

const DEALER: &[&str] = &[
    "dashboard.view",
    "clients.view",
    "groups.view",
    "symbols.view",
    "symbols.edit",
    "positions.view",
    "positions.forceClose",
    "risk.view",
    "lp.view",
    "lp.reconnect",
    "reports.view",
    "reports.export",
    "audit.view",
];
const RISK: &[&str] = &[
    "dashboard.view",
    "clients.view",
    "groups.view",
    "groups.edit",
    "symbols.view",
    "positions.view",
    "positions.forceClose",
    "risk.view",
    "risk.edit",
    "lp.view",
    "reports.view",
    "reports.export",
    "audit.view",
    "balance.approve",
];
const SUPPORT: &[&str] = &[
    "dashboard.view",
    "clients.view",
    "clients.edit",
    "balance.deposit",
    "balance.withdraw",
    "positions.view",
    "reports.view",
    "audit.view",
];

/// Does `role` hold `permission`? Unknown permissions are denied.
pub fn can(role: Role, permission: &str) -> bool {
    if !PERMISSIONS.contains(&permission) {
        return false;
    }
    match role {
        Role::Admin => true,
        Role::Dealer => DEALER.contains(&permission),
        Role::Risk => RISK.contains(&permission),
        Role::Support => SUPPORT.contains(&permission),
        Role::Readonly => {
            permission.ends_with(".view")
                && permission != "users.view"
                && permission != "settings.view"
        }
    }
}

/// Token claims (signing side: tests, tooling, dev tokens).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default)]
    pub name: Option<String>,
    pub role: Role,
    pub exp: u64,
    #[serde(default)]
    pub iat: Option<u64>,
    /// Authentication methods (RFC 8176).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub amr: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
}

/// Claims as accepted on verification: either a single `role` (legacy
/// back-office tokens) or the identity service `roles` array.
#[derive(Deserialize)]
struct RawClaims {
    sub: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    amr: Vec<String>,
    /// Cloudflare Access tokens carry the user's e-mail and no role.
    #[serde(default)]
    email: Option<String>,
}

/// Back-office role of a token: `role`, else the most privileged back-office
/// role in `roles` (`client` alone grants nothing).
fn token_role(c: &RawClaims) -> Option<Role> {
    if let Some(r) = &c.role {
        return Role::parse(r);
    }
    [
        Role::Admin,
        Role::Risk,
        Role::Dealer,
        Role::Support,
        Role::Readonly,
    ]
    .into_iter()
    .find(|r| c.roles.iter().any(|x| x == r.as_str()))
}

/// Authenticated caller, derived from the token only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub sub: String,
    pub name: String,
    pub role: Role,
    /// Token satisfies the MFA policy (true when MFA is not required).
    /// Not journaled.
    #[serde(skip)]
    pub mfa_ok: bool,
}

impl Actor {
    pub fn can(&self, permission: &str) -> bool {
        can(self.role, permission)
    }
}

/// Mutating permissions (everything that is not a `.view`) need MFA when
/// `CORE_REQUIRE_MFA=1`.
pub fn needs_mfa(permission: &str) -> bool {
    !permission.ends_with(".view")
}

#[derive(Debug)]
pub struct AuthError(pub String);

struct Key {
    alg: Algorithm,
    kid: Option<String>,
    key: DecodingKey,
    /// Dev key: validated against [`DEV_ISSUER`] instead of the configured iss/aud.
    dev: bool,
}

struct DevAuth {
    signer: EncodingKey,
    allow_admin: bool,
}

pub struct Authenticator {
    keys: Arc<RwLock<Vec<Key>>>,
    issuer: Option<String>,
    audience: Option<String>,
    require_mfa: bool,
    dev: Option<DevAuth>,
    /// Role by e-mail for tokens without a role claim (Cloudflare Access).
    email_roles: std::collections::HashMap<String, Role>,
    /// Signer for short-lived console session tokens minted from an Access login.
    session: Option<EncodingKey>,
}

/// Startup policy for `CORE_DEV_AUTH=1` (finding G2): never together with a
/// production key source, and only on a loopback bind address.
pub fn dev_auth_policy(dev_auth: bool, has_prod_key: bool, bind: &str) -> Result<(), AuthError> {
    if !dev_auth {
        return Ok(());
    }
    if has_prod_key {
        return Err(AuthError(
            "CORE_DEV_AUTH=1 cannot be combined with CORE_JWT_* keys".into(),
        ));
    }
    let loopback = bind
        .parse::<std::net::SocketAddr>()
        .map(|a| a.ip().is_loopback())
        .unwrap_or(false);
    if !loopback {
        return Err(AuthError(format!(
            "CORE_DEV_AUTH=1 requires a loopback CORE_ADMIN_ADDR (got {bind})"
        )));
    }
    Ok(())
}

fn jwks_keys(set: &JwkSet) -> Result<Vec<Key>, AuthError> {
    let mut out = Vec::new();
    for jwk in &set.keys {
        let alg = match jwk.common.key_algorithm {
            Some(jsonwebtoken::jwk::KeyAlgorithm::RS256) | None => Algorithm::RS256,
            Some(jsonwebtoken::jwk::KeyAlgorithm::ES256) => Algorithm::ES256,
            Some(_) => continue,
        };
        let key = DecodingKey::from_jwk(jwk).map_err(|e| AuthError(e.to_string()))?;
        out.push(Key {
            alg,
            kid: jwk.common.key_id.clone(),
            key,
            dev: false,
        });
    }
    Ok(out)
}

impl Authenticator {
    fn with_keys(keys: Vec<Key>) -> Authenticator {
        Authenticator {
            keys: Arc::new(RwLock::new(keys)),
            issuer: None,
            audience: None,
            require_mfa: false,
            dev: None,
            email_roles: Default::default(),
            session: None,
        }
    }

    pub fn hs256(secret: &[u8]) -> Authenticator {
        Authenticator::with_keys(vec![Key {
            alg: Algorithm::HS256,
            kid: None,
            key: DecodingKey::from_secret(secret),
            dev: false,
        }])
    }

    pub fn rs256_pem(pem: &[u8]) -> Result<Authenticator, AuthError> {
        Ok(Authenticator::with_keys(vec![Key {
            alg: Algorithm::RS256,
            kid: None,
            key: DecodingKey::from_rsa_pem(pem).map_err(|e| AuthError(e.to_string()))?,
            dev: false,
        }]))
    }

    /// RS256/ES256 keys from a JWKS document (identity service).
    pub fn jwks(set: &JwkSet) -> Result<Authenticator, AuthError> {
        Ok(Authenticator::with_keys(jwks_keys(set)?))
    }

    /// Requires `iss` / `aud` on non-dev tokens.
    pub fn with_validation(mut self, issuer: Option<String>, audience: Option<String>) -> Self {
        self.issuer = issuer;
        self.audience = audience;
        self
    }

    /// Requires an MFA `amr` for mutating permissions.
    pub fn with_require_mfa(mut self, on: bool) -> Self {
        self.require_mfa = on;
        self
    }

    /// Adds a dev signer with the given key (tests). See [`Authenticator::with_random_dev_signer`].
    pub fn with_dev_signer(mut self, secret: &[u8], allow_admin: bool) -> Authenticator {
        if let Ok(mut k) = self.keys.write() {
            k.push(Key {
                alg: Algorithm::HS256,
                kid: None,
                key: DecodingKey::from_secret(secret),
                dev: true,
            });
        }
        self.dev = Some(DevAuth {
            signer: EncodingKey::from_secret(secret),
            allow_admin,
        });
        self
    }

    /// Dev signer with a random per-process key: dev tokens die with the process
    /// and no configured secret is ever used to sign them.
    pub fn with_random_dev_signer(self, allow_admin: bool) -> Authenticator {
        use rand::RngCore;
        let mut k = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        self.with_dev_signer(&k, allow_admin)
    }

    /// Maps e-mail addresses (lower-cased) to roles for role-less tokens.
    pub fn with_email_roles(mut self, roles: impl IntoIterator<Item = (String, Role)>) -> Self {
        self.email_roles = roles
            .into_iter()
            .map(|(e, r)| (e.trim().to_lowercase(), r))
            .collect();
        self
    }

    /// Random per-process key for console session tokens ([`Authenticator::sign_session`]).
    /// Verified like dev tokens (local issuer) but does not enable `/auth/dev-token`.
    pub fn with_session_signer(self) -> Authenticator {
        use rand::RngCore;
        let mut secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        if let Ok(mut k) = self.keys.write() {
            k.push(Key {
                alg: Algorithm::HS256,
                kid: None,
                key: DecodingKey::from_secret(&secret),
                dev: true,
            });
        }
        Authenticator {
            session: Some(EncodingKey::from_secret(&secret)),
            ..self
        }
    }

    pub fn session_enabled(&self) -> bool {
        self.session.is_some()
    }

    /// Signs a console session token (only with [`Authenticator::with_session_signer`]).
    pub fn sign_session(&self, claims: &Claims) -> Option<String> {
        let k = self.session.as_ref()?;
        let mut c = claims.clone();
        c.iss = Some(DEV_ISSUER.into());
        encode(&Header::new(Algorithm::HS256), &c, k).ok()
    }

    pub fn dev_enabled(&self) -> bool {
        self.dev.is_some()
    }

    pub fn dev_allows(&self, role: Role) -> bool {
        self.dev
            .as_ref()
            .is_some_and(|d| role != Role::Admin || d.allow_admin)
    }

    /// Replaces the verification keys with a freshly fetched JWKS (keeps dev keys).
    pub fn replace_jwks(&self, set: &JwkSet) -> Result<usize, AuthError> {
        let new = jwks_keys(set)?;
        if new.is_empty() {
            return Err(AuthError("empty JWKS".into()));
        }
        let n = new.len();
        let mut k = self
            .keys
            .write()
            .map_err(|_| AuthError("poisoned".into()))?;
        k.retain(|x| x.dev);
        k.extend(new);
        Ok(n)
    }

    /// Builds the authenticator from the environment (see module docs).
    /// `bind` is the admin listen address (dev auth needs loopback).
    pub fn from_env(dev_auth: bool, bind: &str) -> Result<Authenticator, AuthError> {
        let var = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
        let flag = |k: &str| var(k).is_some_and(|v| v == "1" || v == "true");
        // Cloudflare Access (console behind Tunnel + Access): its JWKS, iss and aud.
        let cf_team = var("CORE_CF_ACCESS_TEAM");
        if let Some(team) = &cf_team {
            dev_auth_policy(dev_auth, true, bind)?;
            let aud = var("CORE_CF_ACCESS_AUD").ok_or_else(|| {
                AuthError(
                    "CORE_CF_ACCESS_TEAM needs CORE_CF_ACCESS_AUD (Access application AUD tag)"
                        .into(),
                )
            })?;
            let admins: Vec<(String, Role)> = var("CORE_CF_ACCESS_ADMINS")
                .unwrap_or_default()
                .split(',')
                .filter(|e| e.contains('@'))
                .map(|e| (e.to_string(), Role::Admin))
                .collect();
            if admins.is_empty() {
                return Err(AuthError(
                    "CORE_CF_ACCESS_TEAM needs CORE_CF_ACCESS_ADMINS (comma-separated e-mails)"
                        .into(),
                ));
            }
            let base = format!(
                "https://{}.cloudflareaccess.com",
                team.trim_end_matches(".cloudflareaccess.com")
            );
            let a = Authenticator::with_keys(vec![]);
            a.spawn_jwks_refresh(
                format!("{base}/cdn-cgi/access/certs"),
                Duration::from_secs(300),
            );
            return Ok(a
                .with_validation(Some(base), Some(aud))
                .with_email_roles(admins)
                .with_session_signer());
        }
        let jwks_url = var("CORE_JWT_JWKS_URL");
        let jwks_file = var("CORE_JWT_JWKS_FILE");
        let rsa = var("CORE_JWT_RS256_PUBLIC_KEY_FILE");
        let secret = var("CORE_JWT_HS256_SECRET");
        let has_prod =
            jwks_url.is_some() || jwks_file.is_some() || rsa.is_some() || secret.is_some();
        dev_auth_policy(dev_auth, has_prod, bind)?;
        let read = |p: &str| std::fs::read(p).map_err(|e| AuthError(format!("{p}: {e}")));
        let base = if let Some(url) = &jwks_url {
            let a = Authenticator::with_keys(vec![]);
            let every = var("CORE_JWT_JWKS_REFRESH_SECS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300);
            a.spawn_jwks_refresh(url.clone(), Duration::from_secs(every));
            a
        } else if let Some(p) = &jwks_file {
            let set: JwkSet =
                serde_json::from_slice(&read(p)?).map_err(|e| AuthError(e.to_string()))?;
            Authenticator::jwks(&set)?
        } else if let Some(p) = &rsa {
            Authenticator::rs256_pem(&read(p)?)?
        } else if let Some(s) = &secret {
            Authenticator::hs256(s.as_bytes())
        } else if dev_auth {
            tracing::warn!("CORE_DEV_AUTH=1: development tokens only, never use in production");
            return Ok(Authenticator::with_keys(vec![])
                .with_random_dev_signer(flag("CORE_DEV_AUTH_ADMIN")));
        } else {
            return Err(AuthError(
                "no JWT key: set CORE_JWT_JWKS_URL (identity), CORE_JWT_JWKS_FILE, \
                 CORE_JWT_RS256_PUBLIC_KEY_FILE or CORE_JWT_HS256_SECRET \
                 (or CORE_DEV_AUTH=1 on a loopback address for development)"
                    .into(),
            ));
        };
        let (iss, aud) = (var("CORE_JWT_ISSUER"), var("CORE_JWT_AUDIENCE"));
        if iss.is_none() || aud.is_none() {
            tracing::warn!("CORE_JWT_ISSUER / CORE_JWT_AUDIENCE not set: iss/aud are not checked");
        }
        Ok(base
            .with_validation(iss, aud)
            .with_require_mfa(flag("CORE_REQUIRE_MFA")))
    }

    /// Fetches `url` now (retrying until it works) and then every `every`.
    /// Needs a Tokio runtime; no-op without one.
    fn spawn_jwks_refresh(&self, url: String, every: Duration) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let keys = Arc::downgrade(&self.keys);
        rt.spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default();
            loop {
                let Some(k) = keys.upgrade() else { return };
                let a = Authenticator {
                    keys: k,
                    issuer: None,
                    audience: None,
                    require_mfa: false,
                    dev: None,
                    email_roles: Default::default(),
                    session: None,
                };
                let ok = match fetch_jwks(&client, &url).await {
                    Ok(set) => a.replace_jwks(&set).map(|_| ()),
                    Err(e) => Err(e),
                };
                drop(a);
                let wait = match ok {
                    Ok(()) => every,
                    Err(e) => {
                        tracing::warn!(%url, error = %e.0, "JWKS refresh failed");
                        Duration::from_secs(5).min(every)
                    }
                };
                tokio::time::sleep(wait).await;
            }
        });
    }

    /// Fetches the JWKS once (tests / startup checks).
    pub async fn refresh_from(&self, url: &str) -> Result<usize, AuthError> {
        let set = fetch_jwks(&reqwest::Client::new(), url).await?;
        self.replace_jwks(&set)
    }

    pub fn verify(&self, token: &str) -> Result<Actor, AuthError> {
        let header = decode_header(token).map_err(|e| AuthError(e.to_string()))?;
        let keys = self.keys.read().map_err(|_| AuthError("poisoned".into()))?;
        let mut last = AuthError("no key for token".into());
        for k in keys.iter().filter(|k| k.alg == header.alg) {
            if let (Some(want), Some(have)) = (&header.kid, &k.kid) {
                if want != have {
                    continue;
                }
            }
            let mut v = Validation::new(k.alg);
            v.leeway = LEEWAY_SECS;
            let mut req = vec!["exp", "sub"];
            if k.dev {
                v.set_issuer(&[DEV_ISSUER]);
                req.push("iss");
            } else {
                if let Some(i) = &self.issuer {
                    v.set_issuer(&[i]);
                    req.push("iss");
                }
                match &self.audience {
                    Some(a) => {
                        v.set_audience(&[a]);
                        req.push("aud");
                    }
                    None => v.validate_aud = false,
                }
            }
            v.set_required_spec_claims(&req);
            match decode::<RawClaims>(token, &k.key, &v) {
                Ok(d) => {
                    let c = d.claims;
                    let email = c.email.as_deref().map(str::to_lowercase);
                    let role = token_role(&c)
                        .or_else(|| {
                            email
                                .as_ref()
                                .and_then(|e| self.email_roles.get(e).copied())
                        })
                        .ok_or_else(|| AuthError("no back-office role in token".into()))?;
                    let mfa = c.amr.iter().any(|m| MFA_AMR.contains(&m.as_str()));
                    return Ok(Actor {
                        name: c.name.clone().or(email).unwrap_or_else(|| c.sub.clone()),
                        sub: c.sub,
                        role,
                        mfa_ok: mfa || !self.require_mfa,
                    });
                }
                Err(e) => last = AuthError(e.to_string()),
            }
        }
        Err(last)
    }

    /// Signs a dev token (only when dev auth is enabled).
    pub fn sign_dev(&self, claims: &Claims) -> Option<String> {
        let d = self.dev.as_ref()?;
        let mut c = claims.clone();
        c.iss = Some(DEV_ISSUER.into());
        encode(&Header::new(Algorithm::HS256), &c, &d.signer).ok()
    }
}

async fn fetch_jwks(client: &reqwest::Client, url: &str) -> Result<JwkSet, AuthError> {
    client
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| AuthError(e.to_string()))?
        .json()
        .await
        .map_err(|e| AuthError(e.to_string()))
}

/// Signs an HS256 token (tests and tooling).
pub fn sign_hs256(secret: &[u8], claims: &Claims) -> String {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret),
    )
    .expect("hs256 signing cannot fail")
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(role: Role) -> Claims {
        Claims {
            sub: "u".into(),
            name: None,
            role,
            exp: unix_now() + 60,
            iat: None,
            amr: vec![],
            iss: None,
        }
    }

    #[test]
    fn dev_auth_policy_rules() {
        assert!(dev_auth_policy(false, true, "0.0.0.0:1").is_ok());
        assert!(dev_auth_policy(true, false, "127.0.0.1:8090").is_ok());
        assert!(dev_auth_policy(true, false, "[::1]:8090").is_ok());
        assert!(dev_auth_policy(true, false, "0.0.0.0:8090").is_err());
        assert!(dev_auth_policy(true, false, "10.0.0.5:8090").is_err());
        assert!(dev_auth_policy(true, false, "localhost:8090").is_err());
        assert!(dev_auth_policy(true, true, "127.0.0.1:8090").is_err());
    }

    #[test]
    fn dev_signer_is_separate_and_admin_gated() {
        let prod = Authenticator::hs256(b"prod-secret");
        // A dev token is never valid against a production-only authenticator.
        let dev = Authenticator::with_keys(vec![]).with_random_dev_signer(false);
        let t = dev.sign_dev(&claims(Role::Support)).unwrap();
        assert!(dev.verify(&t).is_ok());
        assert!(prod.verify(&t).is_err());
        assert!(dev.dev_allows(Role::Support) && !dev.dev_allows(Role::Admin));
        let dev2 = Authenticator::with_keys(vec![]).with_random_dev_signer(true);
        assert!(dev2.dev_allows(Role::Admin));
        // Random keys differ per process/instance.
        assert!(dev2.verify(&t).is_err());
        // A production-key token without the dev issuer is not accepted by the dev key.
        let k = b"k";
        let a = Authenticator::with_keys(vec![]).with_dev_signer(k, false);
        assert!(a.verify(&sign_hs256(k, &claims(Role::Admin))).is_err());
    }

    #[test]
    fn iss_aud_leeway_enforced() {
        #[derive(Serialize)]
        struct C<'a> {
            sub: &'a str,
            exp: u64,
            role: &'a str,
            iss: &'a str,
            aud: &'a str,
        }
        let tok = |iss, aud, exp| {
            encode(
                &Header::new(Algorithm::HS256),
                &C {
                    sub: "u",
                    exp,
                    role: "admin",
                    iss,
                    aud,
                },
                &EncodingKey::from_secret(b"k"),
            )
            .unwrap()
        };
        let now = unix_now();
        let a = Authenticator::hs256(b"k")
            .with_validation(Some("https://id".into()), Some("fxvps".into()));
        assert!(a.verify(&tok("https://id", "fxvps", now + 60)).is_ok());
        assert!(a.verify(&tok("https://evil", "fxvps", now + 60)).is_err());
        assert!(a.verify(&tok("https://id", "client-gw", now + 60)).is_err());
        assert!(a.verify(&sign_hs256(b"k", &claims(Role::Admin))).is_err());
        // expired 20 s ago: beyond the 5 s leeway
        assert!(a.verify(&tok("https://id", "fxvps", now - 20)).is_err());
    }

    #[test]
    fn identity_roles_and_mfa() {
        #[derive(Serialize)]
        struct C<'a> {
            sub: &'a str,
            exp: u64,
            roles: Vec<&'a str>,
            amr: Vec<&'a str>,
        }
        let tok = |roles: Vec<&str>, amr: Vec<&str>| {
            encode(
                &Header::new(Algorithm::HS256),
                &C {
                    sub: "u",
                    exp: unix_now() + 60,
                    roles,
                    amr,
                },
                &EncodingKey::from_secret(b"k"),
            )
            .unwrap()
        };
        let a = Authenticator::hs256(b"k").with_require_mfa(true);
        let actor = a.verify(&tok(vec!["client", "risk"], vec!["pwd"])).unwrap();
        assert_eq!(actor.role, Role::Risk);
        assert!(!actor.mfa_ok);
        assert!(
            a.verify(&tok(vec!["client", "dealer"], vec!["pwd", "otp"]))
                .unwrap()
                .mfa_ok
        );
        assert_eq!(
            a.verify(&tok(vec!["support", "admin"], vec![]))
                .unwrap()
                .role,
            Role::Admin
        );
        // client only: no back-office access
        assert!(a.verify(&tok(vec!["client"], vec!["mfa"])).is_err());
        // MFA not required: always ok
        assert!(
            Authenticator::hs256(b"k")
                .verify(&tok(vec!["risk"], vec![]))
                .unwrap()
                .mfa_ok
        );
        assert!(needs_mfa("balance.deposit") && !needs_mfa("clients.view"));
    }
}
