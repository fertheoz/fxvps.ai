# fxvps.ai Client API

Bots and your own apps use the same API as the web terminal.

## 1. Create a key
Terminal → user menu → Security → **API keys**. Choose a permission:

| Permission | Can |
|---|---|
| `read` | subscribe to prices, read accounts, positions, orders and history |
| `trade` | everything in `read`, plus place, modify and close orders |

The secret (`fxk_...`) is shown **once**. The server keeps only its SHA-256 hash. You can optionally restrict the key to a list of IP addresses. You can have up to 10 active keys, and you can revoke any of them at any time.

## 2. Get a token
```bash
curl -X POST https://id.fxvps.ai/v1/api-keys/token -H "X-API-Key: fxk_..."
```
The response is `{ "access_token": "...", "expires_in": 900, "accounts": [...], "scope": "read" }`. The token lasts 15 minutes. Exchange the key again before it expires.

## 3. Connect
`wss://trade.fxvps.ai/ws`: send the token in the first `auth` message, the same way the terminal does. The message types are documented in `crates/client-proto`.

A `read` token gets `forbidden: read-only API key` on any order command.

## Limits
- Per-account order rate limit (same as the terminal).
- An API-key token always has only the `client` role. It cannot reach admin endpoints and cannot create new keys.
