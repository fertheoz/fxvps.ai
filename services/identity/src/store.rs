//! Persistence boundary. [`Store`] is implemented by [`MemoryStore`] (tests, local
//! dev without a database) and [`crate::pg::PgStore`] (production, `DATABASE_URL`).
//!
//! Secrets are never stored in the clear: refresh tokens and one-time tokens are
//! stored as SHA-256 hashes, passwords and recovery codes as argon2id hashes.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("already exists")]
    Conflict,
    #[error("not found")]
    NotFound,
    #[error("backend: {0}")]
    Backend(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    /// Lower-cased, trimmed.
    pub email: String,
    pub password_hash: String,
    pub email_verified: bool,
    pub roles: Vec<String>,
    /// Base32 TOTP secret (pending until `totp_enabled`).
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    /// Last accepted TOTP time step (replay protection).
    pub totp_last_step: i64,
    /// SHA-256 hashes of unused recovery codes.
    pub recovery_codes: Vec<String>,
    pub failed_logins: i32,
    /// Unix seconds; login refused while `now < locked_until`.
    pub locked_until: i64,
    pub created_at: i64,
}

/// Purpose of a [`OneTimeToken`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenKind {
    VerifyEmail,
    PasswordReset,
    /// Password accepted, second factor pending.
    Mfa,
    PasskeyRegistration,
    PasskeyAuthentication,
}

impl TokenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::VerifyEmail => "verify_email",
            TokenKind::PasswordReset => "password_reset",
            TokenKind::Mfa => "mfa",
            TokenKind::PasskeyRegistration => "passkey_reg",
            TokenKind::PasskeyAuthentication => "passkey_auth",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OneTimeToken {
    pub hash: String,
    pub kind: TokenKind,
    pub user_id: String,
    pub expires_at: i64,
    /// Kind-specific payload (e.g. serialized WebAuthn ceremony state).
    pub data: Option<String>,
    /// Wrong-guess counter (MFA step).
    pub attempts: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshToken {
    pub hash: String,
    pub user_id: String,
    /// All tokens descending from one login share a family; reuse of a rotated
    /// token revokes the whole family.
    pub family_id: String,
    pub amr: Vec<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub revoked: bool,
    /// Set once rotated: the token was exchanged and must not be presented again.
    pub rotated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasskeyRecord {
    pub user_id: String,
    /// base64url credential id.
    pub cred_id: String,
    pub name: String,
    /// Serialized `webauthn_rs::prelude::Passkey`.
    pub data: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub ts: i64,
    pub user_id: Option<String>,
    pub event: String,
    pub ip: Option<String>,
    pub detail: String,
}

/// Outcome of [`Store::rotate_refresh`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// Old token consumed; the new one is stored.
    Rotated(RefreshToken),
    /// Token already rotated or revoked: possible theft, family revoked.
    Reused {
        user_id: String,
        family_id: String,
    },
    Invalid,
}

#[async_trait]
pub trait Store: Send + Sync + 'static {
    async fn create_user(&self, u: &User) -> StoreResult<()>;
    async fn user_by_email(&self, email: &str) -> StoreResult<Option<User>>;
    async fn user_by_id(&self, id: &str) -> StoreResult<Option<User>>;
    async fn update_user(&self, u: &User) -> StoreResult<()>;

    async fn put_token(&self, t: &OneTimeToken) -> StoreResult<()>;
    /// Returns the token if it exists and has the right kind (expired tokens are
    /// returned too; the caller checks `expires_at`).
    async fn get_token(&self, hash: &str, kind: TokenKind) -> StoreResult<Option<OneTimeToken>>;
    /// Atomically removes and returns the token.
    async fn take_token(&self, hash: &str, kind: TokenKind) -> StoreResult<Option<OneTimeToken>>;
    async fn bump_token_attempts(&self, hash: &str) -> StoreResult<i32>;
    /// Removes all tokens of `kind` for `user_id` (e.g. older reset links).
    async fn delete_tokens(&self, user_id: &str, kind: TokenKind) -> StoreResult<()>;

    async fn insert_refresh(&self, t: &RefreshToken) -> StoreResult<()>;
    async fn get_refresh(&self, hash: &str) -> StoreResult<Option<RefreshToken>>;
    /// Atomic compare-and-swap: consume `old_hash` (must be live and unrotated) and
    /// insert `new`. Presenting a rotated/revoked token revokes its family.
    async fn rotate_refresh(
        &self,
        old_hash: &str,
        new: &RefreshToken,
        now: i64,
    ) -> StoreResult<Rotation>;
    async fn revoke_family(&self, family_id: &str) -> StoreResult<()>;
    async fn revoke_user_sessions(&self, user_id: &str) -> StoreResult<u64>;

    async fn add_passkey(&self, p: &PasskeyRecord) -> StoreResult<()>;
    async fn passkeys(&self, user_id: &str) -> StoreResult<Vec<PasskeyRecord>>;
    async fn update_passkey(&self, cred_id: &str, data: &str) -> StoreResult<()>;

    async fn link_account(&self, user_id: &str, account_id: &str) -> StoreResult<()>;
    async fn unlink_account(&self, user_id: &str, account_id: &str) -> StoreResult<bool>;
    async fn accounts(&self, user_id: &str) -> StoreResult<Vec<String>>;

    async fn audit(&self, e: &AuditEvent) -> StoreResult<()>;
    async fn audit_events(&self, user_id: Option<&str>, limit: i64)
        -> StoreResult<Vec<AuditEvent>>;
}

/// In-process store. Data is lost on restart.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Mem>,
}

#[derive(Default)]
struct Mem {
    users: HashMap<String, User>,
    by_email: HashMap<String, String>,
    tokens: HashMap<String, OneTimeToken>,
    refresh: HashMap<String, RefreshToken>,
    passkeys: Vec<PasskeyRecord>,
    links: Vec<(String, String)>,
    audit: Vec<AuditEvent>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Mem> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn create_user(&self, u: &User) -> StoreResult<()> {
        let mut m = self.lock();
        if m.by_email.contains_key(&u.email) || m.users.contains_key(&u.id) {
            return Err(StoreError::Conflict);
        }
        m.by_email.insert(u.email.clone(), u.id.clone());
        m.users.insert(u.id.clone(), u.clone());
        Ok(())
    }

    async fn user_by_email(&self, email: &str) -> StoreResult<Option<User>> {
        let m = self.lock();
        Ok(m.by_email
            .get(email)
            .and_then(|id| m.users.get(id))
            .cloned())
    }

    async fn user_by_id(&self, id: &str) -> StoreResult<Option<User>> {
        Ok(self.lock().users.get(id).cloned())
    }

    async fn update_user(&self, u: &User) -> StoreResult<()> {
        let mut m = self.lock();
        match m.users.get_mut(&u.id) {
            Some(x) => {
                *x = u.clone();
                Ok(())
            }
            None => Err(StoreError::NotFound),
        }
    }

    async fn put_token(&self, t: &OneTimeToken) -> StoreResult<()> {
        self.lock().tokens.insert(t.hash.clone(), t.clone());
        Ok(())
    }

    async fn get_token(&self, hash: &str, kind: TokenKind) -> StoreResult<Option<OneTimeToken>> {
        Ok(self
            .lock()
            .tokens
            .get(hash)
            .filter(|t| t.kind == kind)
            .cloned())
    }

    async fn take_token(&self, hash: &str, kind: TokenKind) -> StoreResult<Option<OneTimeToken>> {
        let mut m = self.lock();
        if m.tokens.get(hash).is_some_and(|t| t.kind == kind) {
            return Ok(m.tokens.remove(hash));
        }
        Ok(None)
    }

    async fn bump_token_attempts(&self, hash: &str) -> StoreResult<i32> {
        let mut m = self.lock();
        let t = m.tokens.get_mut(hash).ok_or(StoreError::NotFound)?;
        t.attempts += 1;
        Ok(t.attempts)
    }

    async fn delete_tokens(&self, user_id: &str, kind: TokenKind) -> StoreResult<()> {
        self.lock()
            .tokens
            .retain(|_, t| !(t.user_id == user_id && t.kind == kind));
        Ok(())
    }

    async fn insert_refresh(&self, t: &RefreshToken) -> StoreResult<()> {
        self.lock().refresh.insert(t.hash.clone(), t.clone());
        Ok(())
    }

    async fn get_refresh(&self, hash: &str) -> StoreResult<Option<RefreshToken>> {
        Ok(self.lock().refresh.get(hash).cloned())
    }

    async fn rotate_refresh(
        &self,
        old_hash: &str,
        new: &RefreshToken,
        now: i64,
    ) -> StoreResult<Rotation> {
        let mut m = self.lock();
        let Some(old) = m.refresh.get_mut(old_hash) else {
            return Ok(Rotation::Invalid);
        };
        if old.revoked || old.rotated {
            let (user_id, family_id) = (old.user_id.clone(), old.family_id.clone());
            for t in m.refresh.values_mut().filter(|t| t.family_id == family_id) {
                t.revoked = true;
            }
            return Ok(Rotation::Reused { user_id, family_id });
        }
        if old.expires_at <= now {
            return Ok(Rotation::Invalid);
        }
        old.rotated = true;
        m.refresh.insert(new.hash.clone(), new.clone());
        Ok(Rotation::Rotated(new.clone()))
    }

    async fn revoke_family(&self, family_id: &str) -> StoreResult<()> {
        for t in self
            .lock()
            .refresh
            .values_mut()
            .filter(|t| t.family_id == family_id)
        {
            t.revoked = true;
        }
        Ok(())
    }

    async fn revoke_user_sessions(&self, user_id: &str) -> StoreResult<u64> {
        let mut n = 0;
        for t in self
            .lock()
            .refresh
            .values_mut()
            .filter(|t| t.user_id == user_id && !t.revoked && !t.rotated)
        {
            t.revoked = true;
            n += 1;
        }
        Ok(n)
    }

    async fn add_passkey(&self, p: &PasskeyRecord) -> StoreResult<()> {
        let mut m = self.lock();
        if m.passkeys.iter().any(|x| x.cred_id == p.cred_id) {
            return Err(StoreError::Conflict);
        }
        m.passkeys.push(p.clone());
        Ok(())
    }

    async fn passkeys(&self, user_id: &str) -> StoreResult<Vec<PasskeyRecord>> {
        Ok(self
            .lock()
            .passkeys
            .iter()
            .filter(|p| p.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn update_passkey(&self, cred_id: &str, data: &str) -> StoreResult<()> {
        let mut m = self.lock();
        let p = m
            .passkeys
            .iter_mut()
            .find(|p| p.cred_id == cred_id)
            .ok_or(StoreError::NotFound)?;
        p.data = data.into();
        Ok(())
    }

    async fn link_account(&self, user_id: &str, account_id: &str) -> StoreResult<()> {
        let mut m = self.lock();
        if !m.users.contains_key(user_id) {
            return Err(StoreError::NotFound);
        }
        if let Some((owner, _)) = m.links.iter().find(|(_, a)| a == account_id) {
            return if owner == user_id {
                Ok(())
            } else {
                Err(StoreError::Conflict)
            };
        }
        m.links.push((user_id.into(), account_id.into()));
        Ok(())
    }

    async fn unlink_account(&self, user_id: &str, account_id: &str) -> StoreResult<bool> {
        let mut m = self.lock();
        let before = m.links.len();
        m.links.retain(|(u, a)| !(u == user_id && a == account_id));
        Ok(m.links.len() != before)
    }

    async fn accounts(&self, user_id: &str) -> StoreResult<Vec<String>> {
        let mut v: Vec<String> = self
            .lock()
            .links
            .iter()
            .filter(|(u, _)| u == user_id)
            .map(|(_, a)| a.clone())
            .collect();
        v.sort();
        Ok(v)
    }

    async fn audit(&self, e: &AuditEvent) -> StoreResult<()> {
        self.lock().audit.push(e.clone());
        Ok(())
    }

    async fn audit_events(
        &self,
        user_id: Option<&str>,
        limit: i64,
    ) -> StoreResult<Vec<AuditEvent>> {
        Ok(self
            .lock()
            .audit
            .iter()
            .rev()
            .filter(|e| user_id.is_none() || e.user_id.as_deref() == user_id)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
    }
}

/// Marks a user's e-mail as verified (operator CLI where no SMTP is configured).
/// Returns `Ok(false)` when it already was.
pub async fn verify_email(store: &dyn Store, email: &str) -> Result<bool, String> {
    let email = email.trim().to_lowercase();
    let mut u = store
        .user_by_email(&email)
        .await
        .map_err(|e| format!("{e:?}"))?
        .ok_or_else(|| format!("no user with email {email}"))?;
    if u.email_verified {
        return Ok(false);
    }
    u.email_verified = true;
    store.update_user(&u).await.map_err(|e| format!("{e:?}"))?;
    Ok(true)
}

/// Links a trading account to a user (operator CLI; same as the admin API).
pub async fn link_account(store: &dyn Store, email: &str, account: &str) -> Result<(), String> {
    let account = account.trim();
    if account.is_empty() || account.len() > 64 {
        return Err("account id must be 1..64 characters".into());
    }
    let email = email.trim().to_lowercase();
    let u = store
        .user_by_email(&email)
        .await
        .map_err(|e| format!("{e:?}"))?
        .ok_or_else(|| format!("no user with email {email}"))?;
    store
        .link_account(&u.id, account)
        .await
        .map_err(|e| format!("{e:?}"))
}

/// Adds the `admin` role to an existing user (one-time bootstrap, G14).
/// Returns `Ok(false)` when the user already had it.
pub async fn grant_admin(store: &dyn Store, email: &str) -> Result<bool, String> {
    let email = email.trim().to_lowercase();
    let mut u = store
        .user_by_email(&email)
        .await
        .map_err(|e| format!("{e:?}"))?
        .ok_or_else(|| format!("no user with email {email}"))?;
    if u.roles.iter().any(|r| r == "admin") {
        return Ok(false);
    }
    u.roles.push("admin".into());
    store.update_user(&u).await.map_err(|e| format!("{e:?}"))?;
    Ok(true)
}

#[cfg(test)]
mod grant_admin_tests {
    use super::*;

    #[tokio::test]
    async fn grants_once() {
        let s = MemoryStore::new();
        let u = User {
            id: "u1".into(),
            email: "a@b.c".into(),
            password_hash: String::new(),
            email_verified: false,
            roles: vec!["trader".into()],
            totp_secret: None,
            totp_enabled: false,
            totp_last_step: 0,
            recovery_codes: vec![],
            failed_logins: 0,
            locked_until: 0,
            created_at: 0,
        };
        s.create_user(&u).await.unwrap();
        assert_eq!(grant_admin(&s, " A@B.c ").await, Ok(true));
        assert_eq!(grant_admin(&s, "a@b.c").await, Ok(false));
        assert!(grant_admin(&s, "x@y.z").await.is_err());
        assert_eq!(verify_email(&s, "a@b.c").await, Ok(true));
        assert_eq!(verify_email(&s, "a@b.c").await, Ok(false));
        link_account(&s, "a@b.c", "100001").await.unwrap();
        assert_eq!(s.accounts("u1").await.unwrap(), vec!["100001".to_string()]);
        assert!(link_account(&s, "a@b.c", " ").await.is_err());
    }
}
