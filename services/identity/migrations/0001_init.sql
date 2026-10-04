-- identity: initial schema. Timestamps are unix seconds (BIGINT).
CREATE TABLE users (
    id               TEXT PRIMARY KEY,
    email            TEXT NOT NULL UNIQUE,
    password_hash    TEXT NOT NULL,
    email_verified   BOOLEAN NOT NULL DEFAULT FALSE,
    roles            TEXT[] NOT NULL DEFAULT '{client}',
    totp_secret      TEXT,
    totp_enabled     BOOLEAN NOT NULL DEFAULT FALSE,
    totp_last_step   BIGINT NOT NULL DEFAULT 0,
    recovery_codes   TEXT[] NOT NULL DEFAULT '{}',
    failed_logins    INTEGER NOT NULL DEFAULT 0,
    locked_until     BIGINT NOT NULL DEFAULT 0,
    created_at       BIGINT NOT NULL
);

-- Email verification / password reset / MFA step / WebAuthn ceremony state.
CREATE TABLE one_time_tokens (
    hash        TEXT PRIMARY KEY,           -- sha256(token), hex
    kind        TEXT NOT NULL,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at  BIGINT NOT NULL,
    data        TEXT,
    attempts    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX one_time_tokens_user ON one_time_tokens(user_id, kind);

CREATE TABLE refresh_tokens (
    hash        TEXT PRIMARY KEY,           -- sha256(token), hex
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    family_id   TEXT NOT NULL,
    amr         TEXT[] NOT NULL DEFAULT '{}',
    created_at  BIGINT NOT NULL,
    expires_at  BIGINT NOT NULL,
    revoked     BOOLEAN NOT NULL DEFAULT FALSE,
    rotated     BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX refresh_tokens_family ON refresh_tokens(family_id);
CREATE INDEX refresh_tokens_user ON refresh_tokens(user_id);

CREATE TABLE passkeys (
    cred_id     TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    data        TEXT NOT NULL,
    created_at  BIGINT NOT NULL
);
CREATE INDEX passkeys_user ON passkeys(user_id);

-- A trading account belongs to exactly one user.
CREATE TABLE trading_accounts (
    account_id  TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    linked_at   BIGINT NOT NULL
);
CREATE INDEX trading_accounts_user ON trading_accounts(user_id);

CREATE TABLE audit_log (
    id       BIGSERIAL PRIMARY KEY,
    ts       BIGINT NOT NULL,
    user_id  TEXT,
    event    TEXT NOT NULL,
    ip       TEXT,
    detail   TEXT NOT NULL DEFAULT ''
);
CREATE INDEX audit_log_user ON audit_log(user_id, id);
