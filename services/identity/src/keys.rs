//! RS256 signing keys and the published JWKS.
//!
//! The key ring holds one active signing key plus retired keys that stay in the
//! JWKS (verification only) so tokens issued before a rotation remain valid until
//! they expire. Relying parties (client-gateway) select keys by `kid`.

use std::sync::RwLock;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("key: {0}")]
    Key(String),
}

struct SigningKey {
    kid: String,
    encoding: EncodingKey,
    decoding: DecodingKey,
    jwk: Value,
}

impl SigningKey {
    fn from_rsa(k: &RsaPrivateKey) -> Result<Self, KeyError> {
        let pub_k = k.to_public_key();
        let n = URL_SAFE_NO_PAD.encode(pub_k.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(pub_k.e().to_bytes_be());
        // RFC 7638 thumbprint (members in lexicographic order), truncated.
        let thumb = Sha256::digest(format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#));
        let kid = URL_SAFE_NO_PAD.encode(&thumb[..12]);
        let pem = k
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .map_err(|e| KeyError::Key(e.to_string()))?;
        let encoding =
            EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| KeyError::Key(e.to_string()))?;
        let decoding =
            DecodingKey::from_rsa_components(&n, &e).map_err(|e| KeyError::Key(e.to_string()))?;
        let jwk = json!({"kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid, "n": n, "e": e});
        Ok(SigningKey {
            kid,
            encoding,
            decoding,
            jwk,
        })
    }
}

pub struct KeyRing {
    inner: RwLock<Ring>,
    /// True when the active key was generated at startup (development only).
    pub ephemeral: bool,
}

struct Ring {
    active: SigningKey,
    retired: Vec<SigningKey>,
}

/// Retired keys kept in the JWKS after rotations.
const MAX_RETIRED: usize = 3;

pub fn generate_rsa() -> Result<RsaPrivateKey, KeyError> {
    RsaPrivateKey::new(&mut rand::thread_rng(), 2048).map_err(|e| KeyError::Key(e.to_string()))
}

pub fn parse_private_pem(pem: &str) -> Result<RsaPrivateKey, KeyError> {
    RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
        .map_err(|e| KeyError::Key(format!("RSA private key PEM (PKCS#8 or PKCS#1): {e}")))
}

impl KeyRing {
    /// `active` signs; `retired` are published for verification only.
    pub fn new(
        active: &RsaPrivateKey,
        retired: &[RsaPrivateKey],
        ephemeral: bool,
    ) -> Result<Self, KeyError> {
        Ok(KeyRing {
            inner: RwLock::new(Ring {
                active: SigningKey::from_rsa(active)?,
                retired: retired
                    .iter()
                    .map(SigningKey::from_rsa)
                    .collect::<Result<_, _>>()?,
            }),
            ephemeral,
        })
    }

    /// DEVELOPMENT: a fresh random key; tokens die with the process.
    pub fn ephemeral() -> Result<Self, KeyError> {
        Self::new(&generate_rsa()?, &[], true)
    }

    pub fn active_kid(&self) -> String {
        self.read().active.kid.clone()
    }

    /// Installs `next` as the signing key; the previous one stays in the JWKS.
    pub fn rotate(&self, next: &RsaPrivateKey) -> Result<String, KeyError> {
        let k = SigningKey::from_rsa(next)?;
        let kid = k.kid.clone();
        let mut g = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let old = std::mem::replace(&mut g.active, k);
        g.retired.insert(0, old);
        g.retired.truncate(MAX_RETIRED);
        Ok(kid)
    }

    pub fn jwks(&self) -> Value {
        let g = self.read();
        let keys: Vec<&Value> = std::iter::once(&g.active.jwk)
            .chain(g.retired.iter().map(|k| &k.jwk))
            .collect();
        json!({ "keys": keys })
    }

    pub fn sign<T: Serialize>(&self, claims: &T) -> Result<String, KeyError> {
        let g = self.read();
        let mut h = Header::new(Algorithm::RS256);
        h.kid = Some(g.active.kid.clone());
        jsonwebtoken::encode(&h, claims, &g.active.encoding)
            .map_err(|e| KeyError::Key(e.to_string()))
    }

    /// Verifies a token issued by this ring (any published key) with `v`.
    pub fn verify<T: serde::de::DeserializeOwned>(&self, token: &str, v: &Validation) -> Option<T> {
        let kid = jsonwebtoken::decode_header(token).ok()?.kid?;
        let g = self.read();
        let k = std::iter::once(&g.active)
            .chain(g.retired.iter())
            .find(|k| k.kid == kid)?;
        jsonwebtoken::decode::<T>(token, &k.decoding, v)
            .ok()
            .map(|d| d.claims)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Ring> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }
}
