//! JWT bearer authentication.
//!
//! Key sources, in order of precedence (see [`Authenticator::from_env`]):
//! 1. `FXVPS_JWT_JWKS_FILE` — JWKS JSON (RS256 / ES256 keys selected by `kid`),
//! 2. `FXVPS_JWT_RS256_PUBLIC_KEY_FILE` — PEM public key,
//! 3. `FXVPS_JWT_HS256_SECRET` — shared secret,
//! 4. otherwise the **development-only** [`DEV_HS256_SECRET`] (logged as a warning).

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
}

struct Key {
    kid: Option<String>,
    alg: Algorithm,
    key: DecodingKey,
}

pub struct Authenticator {
    keys: Vec<Key>,
    /// True when running with [`DEV_HS256_SECRET`].
    pub dev_key: bool,
}

impl Authenticator {
    pub fn hs256(secret: &[u8]) -> Self {
        Authenticator {
            keys: vec![Key {
                kid: None,
                alg: Algorithm::HS256,
                key: DecodingKey::from_secret(secret),
            }],
            dev_key: secret == DEV_HS256_SECRET.as_bytes(),
        }
    }

    pub fn rs256_pem(pem: &[u8]) -> Result<Self, AuthError> {
        Ok(Authenticator {
            keys: vec![Key {
                kid: None,
                alg: Algorithm::RS256,
                key: DecodingKey::from_rsa_pem(pem)?,
            }],
            dev_key: false,
        })
    }

    /// Keys from a JWKS document (e.g. fetched from the IdP's `jwks_uri`).
    pub fn jwks(set: &JwkSet) -> Result<Self, AuthError> {
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
        if keys.is_empty() {
            return Err(AuthError::Config("empty JWKS".into()));
        }
        Ok(Authenticator {
            keys,
            dev_key: false,
        })
    }

    pub fn from_env() -> Result<Self, AuthError> {
        let read =
            |p: String| std::fs::read(&p).map_err(|e| AuthError::Config(format!("{p}: {e}")));
        if let Ok(p) = std::env::var("FXVPS_JWT_JWKS_FILE") {
            let set: JwkSet =
                serde_json::from_slice(&read(p)?).map_err(|e| AuthError::Config(e.to_string()))?;
            return Self::jwks(&set);
        }
        if let Ok(p) = std::env::var("FXVPS_JWT_RS256_PUBLIC_KEY_FILE") {
            return Self::rs256_pem(&read(p)?);
        }
        if let Ok(s) = std::env::var("FXVPS_JWT_HS256_SECRET") {
            return Ok(Self::hs256(s.as_bytes()));
        }
        Ok(Self::hs256(DEV_HS256_SECRET.as_bytes()))
    }

    pub fn verify(&self, token: &str) -> Result<Claims, AuthError> {
        let header = decode_header(token)?;
        let key = self
            .keys
            .iter()
            .find(|k| k.alg == header.alg && (header.kid.is_none() || k.kid == header.kid))
            .ok_or_else(|| AuthError::NoKey {
                alg: header.alg,
                kid: header.kid.clone(),
            })?;
        let mut v = jsonwebtoken::Validation::new(key.alg);
        v.leeway = 5;
        v.set_required_spec_claims(&["exp", "sub"]);
        Ok(decode::<Claims>(token, &key.key, &v)?.claims)
    }
}

/// Issues an HS256 token (tests and `--demo` only).
pub fn issue_hs256(secret: &[u8], sub: &str, accounts: &[&str], ttl_secs: u64) -> String {
    let claims = Claims {
        sub: sub.into(),
        exp: domain::now_ns() / 1_000_000_000 + ttl_secs,
        accounts: accounts.iter().map(|s| s.to_string()).collect(),
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
            },
            &EncodingKey::from_secret(b"k1"),
        )
        .unwrap();
        assert!(matches!(a.verify(&hs384), Err(AuthError::NoKey { .. })));
    }

    #[test]
    fn dev_key_is_flagged() {
        assert!(Authenticator::hs256(DEV_HS256_SECRET.as_bytes()).dev_key);
        assert!(!Authenticator::hs256(b"x").dev_key);
    }
}
