//! Small crypto helpers: random tokens, hashing, argon2id, TOTP.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use totp_rs::TOTP;

/// 256-bit random token, base64url.
pub fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

pub fn random_id() -> String {
    let mut b = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut b);
    // RFC 4122 v4 layout so it doubles as the WebAuthn user handle.
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Constant-time equality for secrets.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Clone)]
pub struct Passwords {
    argon: Argon2<'static>,
    /// Hash of a random password, verified against when the user does not exist
    /// so response time does not reveal which emails are registered.
    dummy: String,
}

impl Passwords {
    pub fn new(m_kib: u32, t: u32) -> Self {
        let params = Params::new(m_kib, t, 1, None).unwrap_or_default();
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut p = Passwords {
            argon,
            dummy: String::new(),
        };
        p.dummy = p.hash(&random_token());
        p
    }

    pub fn hash(&self, password: &str) -> String {
        let salt = SaltString::generate(&mut rand::thread_rng());
        self.argon
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .unwrap_or_default()
    }

    pub fn verify(&self, password: &str, hash: &str) -> bool {
        match PasswordHash::new(hash) {
            Ok(h) => self.argon.verify_password(password.as_bytes(), &h).is_ok(),
            Err(_) => false,
        }
    }

    pub fn burn(&self, password: &str) {
        let _ = self.verify(password, &self.dummy.clone());
    }
}

pub const TOTP_STEP: u64 = 30;

pub fn new_totp_secret() -> String {
    let mut b = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut b);
    totp_rs::Secret::Raw(b.to_vec()).to_encoded().to_string()
}

pub fn totp(secret_b32: &str, account: &str) -> Option<TOTP> {
    let raw = totp_rs::Secret::Encoded(secret_b32.to_string())
        .to_bytes()
        .ok()?;
    TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        TOTP_STEP,
        raw,
        Some("fxvps".into()),
        account.to_string(),
    )
    .ok()
}

/// Checks `code` against steps now-1..=now+1 that are newer than `last_step`.
/// Returns the matched step (store it as the new `last_step`).
pub fn totp_check(t: &TOTP, code: &str, now: u64, last_step: i64) -> Option<i64> {
    let cur = (now / TOTP_STEP) as i64;
    let code = code.trim();
    (cur - 1..=cur + 1).find(|&s| {
        s > last_step
            && s >= 0
            && ct_eq(t.generate(s as u64 * TOTP_STEP).as_bytes(), code.as_bytes())
    })
}

/// `xxxxx-xxxxx` recovery code (lower-case base32, ~50 bits).
pub fn recovery_code() -> String {
    const A: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut b = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut b);
    let s: String = b
        .iter()
        .map(|x| A[(*x as usize) % A.len()] as char)
        .collect();
    format!("{}-{}", &s[..5], &s[5..])
}

pub fn normalize_recovery(code: &str) -> String {
    code.trim().to_lowercase().replace(' ', "")
}
