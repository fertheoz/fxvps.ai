# fxvps.ai Client API

Bots and your own apps use the same API as the web terminal.

## 1. Create a key
Terminal → user menu → Security → **API keys**. Choose a permission:

| Permission | Can |
|---|---|
| `read` | subscribe to prices, read accounts, positions, orders, history and terminal preferences |
| `trade` | everything in `read`, plus place, modify and close orders (and save terminal preferences) |

Neither permission can move money or change the account: deposit / withdrawal requests, KYC documents, IB referral links, copy-trading subscriptions, passwords, 2FA, passkeys, sessions and API keys need an interactive login in the terminal.

The secret (`fxk_...`) is shown **once**. The server keeps only its SHA-256 hash. You can optionally restrict the key to a list of IP addresses (IPv4 or IPv6, in any notation; they are stored in canonical form and compared as addresses). You can have up to 10 active keys, and you can revoke any of them at any time.

A password reset and **Sign out everywhere** (`POST /v1/sessions/revoke-all`) revoke all of your API keys as well. Create new keys afterwards.

## 2. Get a token
```bash
curl -X POST https://id.fxvps.ai/v1/api-keys/token -H "X-API-Key: fxk_..."
```
The response is `{ "access_token": "...", "expires_in": 900, "accounts": [...], "scope": "read" }`. The token lasts 15 minutes. Exchange the key again before it expires. This endpoint is rate limited per IP address (`429 rate_limited`), so cache the token instead of exchanging the key for every request.

With the token you can call `GET https://id.fxvps.ai/v1/me` and `GET /v1/accounts`. Every other identity endpoint answers `403 api_key_forbidden`.

## 3. Connect
`wss://trade.fxvps.ai/ws`: send the token in the first `auth` message, the same way the terminal does. The message types are documented in `crates/client-proto`.

A `read` token gets `forbidden: read-only API key` on any order command and on `PrefsSet`.

The account endpoints under `/api/client/*` accept the token for reading (`GET /api/client/me`, statements, copy-trading and IB overviews). Requests that move money or change links (`POST /api/client/funding`, KYC uploads, `ib/link`, `copy/subscribe`, `copy/unsubscribe`) answer `403 api_key_forbidden` for any API key.

## Limits
- Per-account order rate limit (same as the terminal).
- An API-key token always has only the `client` role. It cannot reach admin endpoints, cannot create or revoke keys, and cannot change passwords, 2FA, passkeys or sessions.
