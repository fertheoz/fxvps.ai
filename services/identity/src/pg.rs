//! PostgreSQL [`Store`] (sqlx). Migrations are embedded from `migrations/`.

use async_trait::async_trait;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::Row;

use crate::store::*;

pub struct PgStore {
    pool: PgPool,
}

fn be(e: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(d) = &e {
        if d.code().as_deref() == Some("23505") {
            return StoreError::Conflict;
        }
        if d.code().as_deref() == Some("23503") {
            return StoreError::NotFound;
        }
    }
    StoreError::Backend(e.to_string())
}

fn kind_from(s: &str) -> TokenKind {
    match s {
        "verify_email" => TokenKind::VerifyEmail,
        "password_reset" => TokenKind::PasswordReset,
        "mfa" => TokenKind::Mfa,
        "passkey_reg" => TokenKind::PasskeyRegistration,
        _ => TokenKind::PasskeyAuthentication,
    }
}

fn user(r: &PgRow) -> User {
    User {
        id: r.get("id"),
        email: r.get("email"),
        password_hash: r.get("password_hash"),
        email_verified: r.get("email_verified"),
        roles: r.get("roles"),
        totp_secret: r.get("totp_secret"),
        totp_enabled: r.get("totp_enabled"),
        totp_last_step: r.get("totp_last_step"),
        recovery_codes: r.get("recovery_codes"),
        failed_logins: r.get("failed_logins"),
        locked_until: r.get("locked_until"),
        created_at: r.get("created_at"),
    }
}

fn token(r: &PgRow) -> OneTimeToken {
    OneTimeToken {
        hash: r.get("hash"),
        kind: kind_from(r.get::<&str, _>("kind")),
        user_id: r.get("user_id"),
        expires_at: r.get("expires_at"),
        data: r.get("data"),
        attempts: r.get("attempts"),
    }
}

fn refresh(r: &PgRow) -> RefreshToken {
    RefreshToken {
        hash: r.get("hash"),
        user_id: r.get("user_id"),
        family_id: r.get("family_id"),
        amr: r.get("amr"),
        created_at: r.get("created_at"),
        expires_at: r.get("expires_at"),
        revoked: r.get("revoked"),
        rotated: r.get("rotated"),
    }
}

impl PgStore {
    /// Connects and applies pending migrations.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .map_err(be)?;
        Self::migrate(pool).await
    }

    /// Like [`PgStore::connect`] but inside `schema` (created if missing); used
    /// by tests to get an isolated database per run.
    pub async fn connect_with_schema(url: &str, schema: &str) -> Result<Self, StoreError> {
        if !schema
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(StoreError::Backend("invalid schema name".into()));
        }
        let setup = PgPoolOptions::new()
            .max_connections(1)
            .connect(url)
            .await
            .map_err(be)?;
        sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .execute(&setup)
            .await
            .map_err(be)?;
        setup.close().await;
        let path = format!("SET search_path TO {schema}");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .after_connect(move |conn, _| {
                let path = path.clone();
                Box::pin(async move {
                    sqlx::Executor::execute(conn, path.as_str()).await?;
                    Ok(())
                })
            })
            .connect(url)
            .await
            .map_err(be)?;
        Self::migrate(pool).await
    }

    async fn migrate(pool: PgPool) -> Result<Self, StoreError> {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        Ok(PgStore { pool })
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

const USER_COLS: &str =
    "id, email, password_hash, email_verified, roles, totp_secret, totp_enabled, \
totp_last_step, recovery_codes, failed_logins, locked_until, created_at";

#[async_trait]
impl Store for PgStore {
    async fn create_user(&self, u: &User) -> StoreResult<()> {
        sqlx::query(&format!(
            "INSERT INTO users ({USER_COLS}) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"
        ))
        .bind(&u.id)
        .bind(&u.email)
        .bind(&u.password_hash)
        .bind(u.email_verified)
        .bind(&u.roles)
        .bind(&u.totp_secret)
        .bind(u.totp_enabled)
        .bind(u.totp_last_step)
        .bind(&u.recovery_codes)
        .bind(u.failed_logins)
        .bind(u.locked_until)
        .bind(u.created_at)
        .execute(&self.pool)
        .await
        .map_err(be)?;
        Ok(())
    }

    async fn user_by_email(&self, email: &str) -> StoreResult<Option<User>> {
        let r = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE email = $1"))
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(be)?;
        Ok(r.as_ref().map(user))
    }

    async fn user_by_id(&self, id: &str) -> StoreResult<Option<User>> {
        let r = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(be)?;
        Ok(r.as_ref().map(user))
    }

    async fn update_user(&self, u: &User) -> StoreResult<()> {
        let n = sqlx::query(
            "UPDATE users SET email=$2, password_hash=$3, email_verified=$4, roles=$5, totp_secret=$6, \
             totp_enabled=$7, totp_last_step=$8, recovery_codes=$9, failed_logins=$10, locked_until=$11 \
             WHERE id=$1",
        )
        .bind(&u.id)
        .bind(&u.email)
        .bind(&u.password_hash)
        .bind(u.email_verified)
        .bind(&u.roles)
        .bind(&u.totp_secret)
        .bind(u.totp_enabled)
        .bind(u.totp_last_step)
        .bind(&u.recovery_codes)
        .bind(u.failed_logins)
        .bind(u.locked_until)
        .execute(&self.pool)
        .await
        .map_err(be)?
        .rows_affected();
        if n == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    async fn put_token(&self, t: &OneTimeToken) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO one_time_tokens (hash, kind, user_id, expires_at, data, attempts) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(&t.hash)
        .bind(t.kind.as_str())
        .bind(&t.user_id)
        .bind(t.expires_at)
        .bind(&t.data)
        .bind(t.attempts)
        .execute(&self.pool)
        .await
        .map_err(be)?;
        Ok(())
    }

    async fn get_token(&self, hash: &str, kind: TokenKind) -> StoreResult<Option<OneTimeToken>> {
        let r = sqlx::query("SELECT * FROM one_time_tokens WHERE hash=$1 AND kind=$2")
            .bind(hash)
            .bind(kind.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(be)?;
        Ok(r.as_ref().map(token))
    }

    async fn take_token(&self, hash: &str, kind: TokenKind) -> StoreResult<Option<OneTimeToken>> {
        let r = sqlx::query("DELETE FROM one_time_tokens WHERE hash=$1 AND kind=$2 RETURNING *")
            .bind(hash)
            .bind(kind.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(be)?;
        Ok(r.as_ref().map(token))
    }

    async fn bump_token_attempts(&self, hash: &str) -> StoreResult<i32> {
        let r = sqlx::query(
            "UPDATE one_time_tokens SET attempts = attempts + 1 WHERE hash=$1 RETURNING attempts",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(be)?;
        r.map(|r| r.get(0)).ok_or(StoreError::NotFound)
    }

    async fn delete_tokens(&self, user_id: &str, kind: TokenKind) -> StoreResult<()> {
        sqlx::query("DELETE FROM one_time_tokens WHERE user_id=$1 AND kind=$2")
            .bind(user_id)
            .bind(kind.as_str())
            .execute(&self.pool)
            .await
            .map_err(be)?;
        Ok(())
    }

    async fn insert_refresh(&self, t: &RefreshToken) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO refresh_tokens (hash, user_id, family_id, amr, created_at, expires_at, revoked, rotated) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(&t.hash)
        .bind(&t.user_id)
        .bind(&t.family_id)
        .bind(&t.amr)
        .bind(t.created_at)
        .bind(t.expires_at)
        .bind(t.revoked)
        .bind(t.rotated)
        .execute(&self.pool)
        .await
        .map_err(be)?;
        Ok(())
    }

    async fn get_refresh(&self, hash: &str) -> StoreResult<Option<RefreshToken>> {
        let r = sqlx::query("SELECT * FROM refresh_tokens WHERE hash=$1")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(be)?;
        Ok(r.as_ref().map(refresh))
    }

    async fn rotate_refresh(
        &self,
        old_hash: &str,
        new: &RefreshToken,
        now: i64,
    ) -> StoreResult<Rotation> {
        let mut tx = self.pool.begin().await.map_err(be)?;
        let consumed = sqlx::query(
            "UPDATE refresh_tokens SET rotated = TRUE \
             WHERE hash=$1 AND NOT revoked AND NOT rotated AND expires_at > $2 RETURNING hash",
        )
        .bind(old_hash)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(be)?;
        if consumed.is_none() {
            let old = sqlx::query("SELECT * FROM refresh_tokens WHERE hash=$1")
                .bind(old_hash)
                .fetch_optional(&mut *tx)
                .await
                .map_err(be)?
                .as_ref()
                .map(refresh);
            let out = match old {
                Some(o) if o.revoked || o.rotated => {
                    sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE family_id=$1")
                        .bind(&o.family_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(be)?;
                    Rotation::Reused {
                        user_id: o.user_id,
                        family_id: o.family_id,
                    }
                }
                _ => Rotation::Invalid,
            };
            tx.commit().await.map_err(be)?;
            return Ok(out);
        }
        sqlx::query(
            "INSERT INTO refresh_tokens (hash, user_id, family_id, amr, created_at, expires_at, revoked, rotated) \
             VALUES ($1,$2,$3,$4,$5,$6,FALSE,FALSE)",
        )
        .bind(&new.hash)
        .bind(&new.user_id)
        .bind(&new.family_id)
        .bind(&new.amr)
        .bind(new.created_at)
        .bind(new.expires_at)
        .execute(&mut *tx)
        .await
        .map_err(be)?;
        tx.commit().await.map_err(be)?;
        Ok(Rotation::Rotated(new.clone()))
    }

    async fn revoke_family(&self, family_id: &str) -> StoreResult<()> {
        sqlx::query("UPDATE refresh_tokens SET revoked = TRUE WHERE family_id=$1")
            .bind(family_id)
            .execute(&self.pool)
            .await
            .map_err(be)?;
        Ok(())
    }

    async fn revoke_user_sessions(&self, user_id: &str) -> StoreResult<u64> {
        Ok(sqlx::query(
            "UPDATE refresh_tokens SET revoked = TRUE WHERE user_id=$1 AND NOT revoked AND NOT rotated",
        )
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(be)?
        .rows_affected())
    }

    async fn add_passkey(&self, p: &PasskeyRecord) -> StoreResult<()> {
        sqlx::query("INSERT INTO passkeys (cred_id, user_id, name, data, created_at) VALUES ($1,$2,$3,$4,$5)")
            .bind(&p.cred_id)
            .bind(&p.user_id)
            .bind(&p.name)
            .bind(&p.data)
            .bind(p.created_at)
            .execute(&self.pool)
            .await
            .map_err(be)?;
        Ok(())
    }

    async fn passkeys(&self, user_id: &str) -> StoreResult<Vec<PasskeyRecord>> {
        let rows = sqlx::query("SELECT * FROM passkeys WHERE user_id=$1 ORDER BY created_at")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(be)?;
        Ok(rows
            .iter()
            .map(|r| PasskeyRecord {
                user_id: r.get("user_id"),
                cred_id: r.get("cred_id"),
                name: r.get("name"),
                data: r.get("data"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    async fn update_passkey(&self, cred_id: &str, data: &str) -> StoreResult<()> {
        sqlx::query("UPDATE passkeys SET data=$2 WHERE cred_id=$1")
            .bind(cred_id)
            .bind(data)
            .execute(&self.pool)
            .await
            .map_err(be)?;
        Ok(())
    }

    async fn link_account(&self, user_id: &str, account_id: &str) -> StoreResult<()> {
        let r = sqlx::query(
            "INSERT INTO trading_accounts (account_id, user_id, linked_at) \
             VALUES ($1, $2, EXTRACT(EPOCH FROM now())::BIGINT) \
             ON CONFLICT (account_id) DO NOTHING",
        )
        .bind(account_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(be)?;
        if r.rows_affected() == 1 {
            return Ok(());
        }
        let owner: Option<String> =
            sqlx::query_scalar("SELECT user_id FROM trading_accounts WHERE account_id=$1")
                .bind(account_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(be)?;
        if owner.as_deref() == Some(user_id) {
            Ok(())
        } else {
            Err(StoreError::Conflict)
        }
    }

    async fn unlink_account(&self, user_id: &str, account_id: &str) -> StoreResult<bool> {
        Ok(
            sqlx::query("DELETE FROM trading_accounts WHERE user_id=$1 AND account_id=$2")
                .bind(user_id)
                .bind(account_id)
                .execute(&self.pool)
                .await
                .map_err(be)?
                .rows_affected()
                == 1,
        )
    }

    async fn accounts(&self, user_id: &str) -> StoreResult<Vec<String>> {
        sqlx::query_scalar(
            "SELECT account_id FROM trading_accounts WHERE user_id=$1 ORDER BY account_id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(be)
    }

    async fn audit(&self, e: &AuditEvent) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO audit_log (ts, user_id, event, ip, detail) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(e.ts)
        .bind(&e.user_id)
        .bind(&e.event)
        .bind(&e.ip)
        .bind(&e.detail)
        .execute(&self.pool)
        .await
        .map_err(be)?;
        Ok(())
    }

    async fn audit_events(
        &self,
        user_id: Option<&str>,
        limit: i64,
    ) -> StoreResult<Vec<AuditEvent>> {
        let rows = sqlx::query(
            "SELECT ts, user_id, event, ip, detail FROM audit_log \
             WHERE ($1::TEXT IS NULL OR user_id = $1) ORDER BY id DESC LIMIT $2",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(be)?;
        Ok(rows
            .iter()
            .map(|r| AuditEvent {
                ts: r.get("ts"),
                user_id: r.get("user_id"),
                event: r.get("event"),
                ip: r.get("ip"),
                detail: r.get("detail"),
            })
            .collect())
    }

    async fn put_api_key(&self, k: &ApiKey) -> StoreResult<()> {
        sqlx::query("INSERT INTO api_keys (id, user_id, name, hash, scope, ips, created_at, last_used, revoked) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(&k.id)
            .bind(&k.user_id)
            .bind(&k.name)
            .bind(&k.hash)
            .bind(&k.scope)
            .bind(&k.ips)
            .bind(k.created_at)
            .bind(k.last_used)
            .bind(k.revoked)
            .execute(&self.pool)
            .await
            .map_err(be)?;
        Ok(())
    }

    async fn api_keys(&self, user_id: &str) -> StoreResult<Vec<ApiKey>> {
        let rows = sqlx::query("SELECT * FROM api_keys WHERE user_id=$1 ORDER BY created_at")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(be)?;
        Ok(rows.iter().map(api_key).collect())
    }

    async fn api_key_by_hash(&self, hash: &str) -> StoreResult<Option<ApiKey>> {
        let r = sqlx::query("SELECT * FROM api_keys WHERE hash=$1")
            .bind(hash)
            .fetch_optional(&self.pool)
            .await
            .map_err(be)?;
        Ok(r.as_ref().map(api_key))
    }

    async fn revoke_api_key(&self, user_id: &str, id: &str) -> StoreResult<bool> {
        Ok(
            sqlx::query("UPDATE api_keys SET revoked = TRUE WHERE id=$1 AND user_id=$2")
                .bind(id)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(be)?
                .rows_affected()
                > 0,
        )
    }

    async fn touch_api_key(&self, id: &str, ts: i64) -> StoreResult<()> {
        sqlx::query("UPDATE api_keys SET last_used=$2 WHERE id=$1")
            .bind(id)
            .bind(ts)
            .execute(&self.pool)
            .await
            .map_err(be)?;
        Ok(())
    }
}

fn api_key(r: &PgRow) -> ApiKey {
    ApiKey {
        id: r.get("id"),
        user_id: r.get("user_id"),
        name: r.get("name"),
        hash: r.get("hash"),
        scope: r.get("scope"),
        ips: r.get("ips"),
        created_at: r.get("created_at"),
        last_used: r.get("last_used"),
        revoked: r.get("revoked"),
    }
}
