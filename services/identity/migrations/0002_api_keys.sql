-- Personal API keys: only the SHA-256 of the secret is stored.
CREATE TABLE IF NOT EXISTS api_keys (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    hash        TEXT NOT NULL UNIQUE,
    scope       TEXT NOT NULL,
    ips         TEXT[] NOT NULL DEFAULT '{}',
    created_at  BIGINT NOT NULL,
    last_used   BIGINT,
    revoked     BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX IF NOT EXISTS api_keys_user ON api_keys (user_id);
