//! JWT bearer authentication and the role/permission matrix of the back
//! office (mirrors `apps/backoffice/src/lib/rbac.ts`; the server is the
//! authority, the UI guards are UX only).
//!
//! Key sources (see [`Authenticator::from_env`]):
//! - `CORE_JWT_RS256_PUBLIC_KEY_FILE`: PEM public key (RS256, IdP-issued tokens),
//! - `CORE_JWT_HS256_SECRET`: shared secret (HS256),
//! - `CORE_DEV_AUTH=1` without a secret: the public, insecure
//!   [`DEV_HS256_SECRET`] (development only, logged as a warning).

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// DEVELOPMENT ONLY. Public and insecure; accepted only with `CORE_DEV_AUTH=1`.
pub const DEV_HS256_SECRET: &str = "core-engine-DEV-ONLY-insecure-hs256-key-not-for-production";

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

pub const PERMISSIONS: [&str; 24] = [
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

/// Token claims.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    #[serde(default)]
    pub name: Option<String>,
    pub role: Role,
    pub exp: u64,
    #[serde(default)]
    pub iat: Option<u64>,
}

/// Authenticated caller, derived from the token only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub sub: String,
    pub name: String,
    pub role: Role,
}

impl Actor {
    pub fn can(&self, permission: &str) -> bool {
        can(self.role, permission)
    }
}

#[derive(Debug)]
pub struct AuthError(pub String);

struct Key {
    alg: Algorithm,
    key: DecodingKey,
}

pub struct Authenticator {
    keys: Vec<Key>,
    /// HS256 signing key for `POST /auth/dev-token` (dev auth only).
    dev_signer: Option<EncodingKey>,
}

impl Authenticator {
    pub fn hs256(secret: &[u8]) -> Authenticator {
        Authenticator {
            keys: vec![Key {
                alg: Algorithm::HS256,
                key: DecodingKey::from_secret(secret),
            }],
            dev_signer: None,
        }
    }

    pub fn rs256_pem(pem: &[u8]) -> Result<Authenticator, AuthError> {
        Ok(Authenticator {
            keys: vec![Key {
                alg: Algorithm::RS256,
                key: DecodingKey::from_rsa_pem(pem).map_err(|e| AuthError(e.to_string()))?,
            }],
            dev_signer: None,
        })
    }

    /// Adds an HS256 key that also signs dev tokens.
    pub fn with_dev_signer(mut self, secret: &[u8]) -> Authenticator {
        self.keys.push(Key {
            alg: Algorithm::HS256,
            key: DecodingKey::from_secret(secret),
        });
        self.dev_signer = Some(EncodingKey::from_secret(secret));
        self
    }

    pub fn dev_enabled(&self) -> bool {
        self.dev_signer.is_some()
    }

    /// Builds the authenticator from the environment (see module docs).
    pub fn from_env(dev_auth: bool) -> Result<Authenticator, AuthError> {
        let secret = std::env::var("CORE_JWT_HS256_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let rsa = std::env::var("CORE_JWT_RS256_PUBLIC_KEY_FILE")
            .ok()
            .filter(|s| !s.is_empty());
        let base = match (&rsa, &secret) {
            (Some(path), _) => {
                let pem = std::fs::read(path).map_err(|e| AuthError(format!("{path}: {e}")))?;
                Some(Authenticator::rs256_pem(&pem)?)
            }
            (None, Some(s)) if !dev_auth => Some(Authenticator::hs256(s.as_bytes())),
            _ => None,
        };
        if dev_auth {
            let s = secret.unwrap_or_else(|| {
                tracing::warn!(
                    "CORE_DEV_AUTH=1 with the public DEV secret: never use in production"
                );
                DEV_HS256_SECRET.into()
            });
            let base = base.unwrap_or(Authenticator {
                keys: vec![],
                dev_signer: None,
            });
            return Ok(base.with_dev_signer(s.as_bytes()));
        }
        base.ok_or_else(|| {
            AuthError(
                "no JWT key: set CORE_JWT_RS256_PUBLIC_KEY_FILE or CORE_JWT_HS256_SECRET \
                 (or CORE_DEV_AUTH=1 for development)"
                    .into(),
            )
        })
    }

    pub fn verify(&self, token: &str) -> Result<Actor, AuthError> {
        let mut last = AuthError("no key".into());
        for k in &self.keys {
            let mut v = Validation::new(k.alg);
            v.set_required_spec_claims(&["exp", "sub"]);
            v.leeway = 30;
            match decode::<Claims>(token, &k.key, &v) {
                Ok(d) => {
                    let c = d.claims;
                    return Ok(Actor {
                        name: c.name.clone().unwrap_or_else(|| c.sub.clone()),
                        sub: c.sub,
                        role: c.role,
                    });
                }
                Err(e) => last = AuthError(e.to_string()),
            }
        }
        Err(last)
    }

    /// Signs a dev token (only when dev auth is enabled).
    pub fn sign_dev(&self, claims: &Claims) -> Option<String> {
        let key = self.dev_signer.as_ref()?;
        encode(&Header::new(Algorithm::HS256), claims, key).ok()
    }
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
