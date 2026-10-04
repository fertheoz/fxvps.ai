# identity

Identity and authentication for fxvps.ai: users, passwords, email verification,
password reset, TOTP 2FA with recovery codes, passkeys (WebAuthn), rotating refresh
sessions, and short-lived **RS256 access tokens** that `client-gateway` (and later
core-engine / back office) verify through the published JWKS.

```
cargo run -p identity                     # dev: in-memory store, ephemeral key, log mailer
DATABASE_URL=postgres://... cargo run -p identity
docker build -f infra/docker/identity.Dockerfile -t fxvps-identity .
```

On startup the service prints `FXVPS_IDENTITY_URL=http://<addr>` (scripts / e2e).

## Configuration (environment)

| Variable | Default | |
|---|---|---|
| `IDENTITY_LISTEN` | `127.0.0.1:8090` | listen address |
| `IDENTITY_ISSUER` | `http://127.0.0.1:8090` | `iss` claim, discovery base URL |
| `IDENTITY_AUDIENCE` | `fxvps` | `aud` claim |
| `DATABASE_URL` | — | PostgreSQL; **unset = in-memory store (dev only)**. Migrations (`migrations/`) run at startup. |
| `IDENTITY_SIGNING_KEY_FILE` | — | RSA private key PEM (PKCS#8/PKCS#1). **Unset = ephemeral DEVELOPMENT key** generated at startup (warned in the log; tokens die with the process). |
| `IDENTITY_RETIRED_KEY_FILES` | — | comma separated PEMs still published in the JWKS (rotation) |
| `IDENTITY_RP_ID` / `IDENTITY_RP_ORIGIN` | `localhost` / `http://localhost:5173` | WebAuthn relying party |
| `IDENTITY_ALLOWED_ORIGINS` | `http://localhost:5173,http://127.0.0.1:5173` | CORS + cookie-mode Origin allow list |
| `IDENTITY_PUBLIC_URL` | `IDENTITY_RP_ORIGIN` | base of emailed links (`/?verify_email=…`, `/?reset_password=…`) |
| `IDENTITY_SERVICE_TOKEN` | — | static bearer (≥ 32 chars) accepted on `/v1/admin/*` for service-to-service calls |
| `IDENTITY_BOOTSTRAP_ADMINS` | — | emails that get the `admin` role on registration |
| `IDENTITY_COOKIE_SECURE` | `true` | `Secure` flag on the refresh cookie |
| `IDENTITY_TRUST_PROXY` | `false` | client IP from `X-Forwarded-For` (behind a trusted proxy only) |
| `IDENTITY_ACCESS_TTL_SECS` / `IDENTITY_REFRESH_TTL_SECS` | 300 / 30 d | token lifetimes |
| `IDENTITY_IP_REQUESTS_PER_MINUTE` | 60 | per-IP limit on the auth endpoints |
| `IDENTITY_DEV_MAIL_FILE` | — | dev mailer also appends each mail as a JSON line (e2e tests) |

No secret has a default and none is committed. Email delivery goes through the
`Mailer` trait; the shipped `LogMailer` only logs (development). A production
mailer (SMTP / provider API) plugs in at `main.rs`.

## Tokens

Access token (JWT, RS256, header `kid`), 5 minutes:

```json
{ "iss": "...", "aud": "fxvps", "sub": "<user uuid>", "exp": 0, "iat": 0, "jti": "...",
  "sid": "<session id>", "email": "...", "accounts": ["ACC-1"],
  "roles": ["client"], "amr": ["pwd", "mfa", "otp"] }
```

* `roles` ⊆ `client | admin | dealer | risk | support | readonly`.
* `amr` (RFC 8176): `pwd` password, `otp` TOTP, `rc` recovery code, `hwk` passkey, `mfa` multi-factor.
* `GET /.well-known/jwks.json` — active + retired public keys (`Cache-Control: max-age=300`).
* `GET /.well-known/openid-configuration` — discovery document.

Refresh tokens are opaque 256-bit values stored as SHA-256 hashes. Every refresh
**rotates** the token; presenting an already-rotated token is treated as theft and
revokes the whole session (token family). Password reset revokes all sessions.

**Web (cookie mode)** — send `"session": "cookie"` on login: the refresh token is set
as `fxvps_rt` (`HttpOnly; Secure; SameSite=Strict; Path=/v1/token`) and never appears
in the body. Cookie-authenticated calls (`/v1/token/refresh`, `/v1/token/revoke`)
must carry `X-Fxvps-Csrf: 1` (forces a CORS preflight) and, when present, an allowed
`Origin`. **Desktop / mobile (bearer mode, default)** — the refresh token is returned
in the body and sent back in the JSON body.

## HTTP API

Errors: `{"error": "<code>", "message": "..."}`.

| Method & path | Auth | |
|---|---|---|
| `POST /v1/register` `{email,password}` | — | always `202` (no account enumeration); sends the verification link |
| `POST /v1/verify-email` `{token}` | — | |
| `POST /v1/verify-email/resend` `{email}` | — | `202` |
| `POST /v1/password/forgot` `{email}` | — | `202` |
| `POST /v1/password/reset` `{token,password}` | — | clears lockout, revokes sessions |
| `POST /v1/login` `{email,password,session?}` | — | tokens, or `{mfa_required, mfa_token, methods}` |
| `POST /v1/login/2fa` `{mfa_token, code \| recovery_code, session?}` | — | tokens |
| `POST /v1/passkeys/login/start` `{email}` → `{authentication_id, options}` | — | |
| `POST /v1/passkeys/login/finish` `{authentication_id, credential, session?}` | — | tokens (`amr: hwk,mfa`) |
| `POST /v1/token/refresh` `{refresh_token?}` | refresh | rotate |
| `POST /v1/token/revoke` `{refresh_token?}` | refresh | logout (`204`) |
| `GET /v1/me`, `GET /v1/accounts` | bearer | profile / trading accounts |
| `POST /v1/sessions/revoke-all` | bearer | |
| `POST /v1/2fa/totp/enroll` → `{secret, otpauth_url}` | bearer | |
| `POST /v1/2fa/totp/confirm` `{code}` → `{recovery_codes}` | bearer | 10 single-use codes, shown once |
| `POST /v1/2fa/totp/disable` `{code}` | bearer | |
| `POST /v1/passkeys/register/start` / `finish` | bearer | |
| `GET /v1/admin/users?email=\|id=` | admin | |
| `POST /v1/admin/users/roles` `{user_id\|email, roles}` | admin | |
| `POST /v1/admin/accounts/link` `{user_id\|email, account_id}` | admin | idempotent; `409` if another user owns it |
| `POST /v1/admin/accounts/unlink` | admin | |
| `GET /v1/admin/audit?user_id=&limit=` | admin | audit log |
| `POST /v1/admin/keys/rotate` | admin | new in-memory signing key (old stays in JWKS) |

`admin` = an access token with the `admin` role **or** `Authorization: Bearer $IDENTITY_SERVICE_TOKEN`.

## Brute-force protection

* Per-IP token bucket on all unauthenticated auth endpoints (`429 rate_limited`).
* Per-account lockout: 5 failed passwords / second factors → `423 account_locked`
  for 15 minutes. Unknown emails get the same `401` and a dummy argon2 verification.
* TOTP codes are single use (last accepted time step is stored); MFA step tokens
  expire after 5 minutes and die after 5 wrong codes.
* argon2id (19 MiB, t=2) for passwords.

## Audit log

Every auth event (`register`, `email_verified`, `login_success`, `login_failed`,
`account_locked`, `login_mfa_required`, `token_refreshed`, `refresh_token_reuse`,
`logout`, `sessions_revoked`, `password_reset_requested`, `password_reset`,
`totp_enabled`, `totp_disabled`, `recovery_code_used`, `passkey_registered`,
`roles_changed`, `account_linked`, `account_unlinked`, `signing_key_rotated`,
`rate_limited`) is written to `audit_log` with user id, client IP and detail, and to
the `audit` tracing target.

## Relying parties

**client-gateway** (`services/client-gateway/src/auth.rs`):

```
FXVPS_JWT_JWKS_URL=https://id.fxvps.ai/.well-known/jwks.json
FXVPS_JWT_ISSUER=https://id.fxvps.ai        # = IDENTITY_ISSUER
FXVPS_JWT_AUDIENCE=fxvps                    # = IDENTITY_AUDIENCE
# optional: FXVPS_JWT_JWKS_REFRESH_SECS=300, FXVPS_JWT_JWKS_MIN_REFRESH_SECS=10
```

The gateway caches the JWKS, refreshes it periodically and immediately (rate
limited) when a token carries an unknown `kid`, so key rotation needs no restart.

**Terminal** (`apps/terminal`): build with `VITE_IDENTITY_URL=https://id.fxvps.ai`.
`?api=ws&url=wss://…` without a pasted dev token then shows the login screen;
the access token lives in memory and is refreshed silently from the cookie.

**core-engine / back office (planned, not wired yet).** The back office keeps its
own operator login for now; to move onto identity:

1. Operators are identity users with `admin`, `dealer`, `risk`, `support` or
   `readonly` roles (assigned with `POST /v1/admin/users/roles`; the first admin via
   `IDENTITY_BOOTSTRAP_ADMINS`). The back office verifies access tokens exactly like
   client-gateway (JWKS URL + `iss`/`aud`) and maps `roles` onto its permissions;
   require `amr` to contain `mfa` for operator actions.
2. When core-engine / the back office opens a trading account for a client it calls
   `POST /v1/admin/accounts/link {"email" | "user_id", "account_id"}` with
   `Authorization: Bearer $IDENTITY_SERVICE_TOKEN` (idempotent, safe to retry);
   closing an account calls `/v1/admin/accounts/unlink`. The client's next token
   refresh (≤ 5 min) carries the new `accounts` claim.
3. Support lookups: `GET /v1/admin/users?email=` and `GET /v1/admin/audit?user_id=`.

## Tests

```
cargo test -p identity                       # in-memory store
DATABASE_URL=postgres://u:p@localhost/db cargo test -p identity   # + PostgreSQL (fresh schema per run)
cargo test -p client-gateway --test identity_jwks
```

Passkey ceremonies are covered up to the WebAuthn library boundary (options,
state persistence, rejection of bad credentials); full authenticator round trips
need a browser (manual / future virtual-authenticator e2e).
