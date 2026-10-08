# fxvps.ai Client API

Bots and your own apps use the same API as the web terminal: a **REST API** (`https://trade.fxvps.ai/api/v1`) for accounts, orders and history, and the **WebSocket** (`wss://trade.fxvps.ai/ws`) for streaming prices and account events. Both take the same token and enforce the same rules.

The OpenAPI 3 description of the REST API is in [`openapi.yaml`](openapi.yaml).

## 1. Create a key
Terminal → user menu → Security → **API keys**. Choose a permission:

| Permission | Can |
|---|---|
| `read` | subscribe to prices, read accounts, positions, orders, history and terminal preferences |
| `trade` | everything in `read`, plus place, modify and close orders (and save terminal preferences) |
| `read` | subscribe to prices, read accounts, positions, orders and history |
| `trade` | everything in `read`, plus place, modify, cancel and close orders |

Neither permission can move money or change the account: deposit / withdrawal requests, KYC documents, IB referral links, copy-trading subscriptions, passwords, 2FA, passkeys, sessions and API keys need an interactive login in the terminal.

The secret (`fxk_...`) is shown **once**. The server keeps only its SHA-256 hash. You can optionally restrict the key to a list of IP addresses (IPv4 or IPv6, in any notation; they are stored in canonical form and compared as addresses). You can have up to 10 active keys, and you can revoke any of them at any time.

A password reset and **Sign out everywhere** (`POST /v1/sessions/revoke-all`) revoke all of your API keys as well. Create new keys afterwards.

## 2. Get a token
```bash
curl -X POST https://id.fxvps.ai/v1/api-keys/token -H "X-API-Key: fxk_..."
```
The response is `{ "access_token": "...", "expires_in": 900, "accounts": [...], "scope": "read" }`. The token lasts 15 minutes. Exchange the key again before it expires. This endpoint is rate limited per IP address (`429 rate_limited`), so cache the token instead of exchanging the key for every request.

With the token you can call `GET https://id.fxvps.ai/v1/me` and `GET /v1/accounts`. Every other identity endpoint answers `403 api_key_forbidden`.
The response is `{ "access_token": "...", "expires_in": 900, "accounts": [...], "scope": "read" }`. The token lasts 15 minutes. Exchange the key again before it expires. A revoked key gets `401`.

The examples below use:
```bash
API=https://trade.fxvps.ai/api/v1
TOKEN=$(curl -s -X POST https://id.fxvps.ai/v1/api-keys/token -H "X-API-Key: $FXK" | jq -r .access_token)
```

## 3. REST API
Every request needs `Authorization: Bearer $TOKEN`. Bodies are JSON.

- **Numbers**: prices and quantities are decimal **strings** (`"1.08512"`). Plain JSON numbers are accepted on input, but strings are exact.
- **Quantity** (`qty`) is in **base units**, as in the terminal protocol: 1 lot = the symbol's `contract_size` (`100000` for FX majors).
- **Account**: if your token holds one trading account you can leave `account` out. With several accounts, pass `account` (query parameter for `GET` and `DELETE`, query or body for the other commands).
- **Times** in responses are nanoseconds since the Unix epoch (`*_ns`). Time parameters (`from`, `to`, `expire_at`) take Unix **milliseconds** or RFC 3339 (`2026-10-01T00:00:00Z`).

### Accounts and market data
```bash
curl -H "Authorization: Bearer $TOKEN" $API/accounts
curl -H "Authorization: Bearer $TOKEN" $API/accounts/100231
curl -H "Authorization: Bearer $TOKEN" $API/symbols
curl -H "Authorization: Bearer $TOKEN" $API/quotes/EURUSD
curl -H "Authorization: Bearer $TOKEN" "$API/candles/EURUSD?timeframe=H1&limit=100"
```
`/accounts` returns balance, equity, margin, free margin and margin level. `/quotes/{symbol}` returns the latest bid and ask of your account's price group.

### Positions, orders, history
```bash
curl -H "Authorization: Bearer $TOKEN" "$API/positions?account=100231"
curl -H "Authorization: Bearer $TOKEN" "$API/orders?account=100231"            # working orders
curl -H "Authorization: Bearer $TOKEN" "$API/orders?account=100231&history=1"  # finished orders, newest first
curl -H "Authorization: Bearer $TOKEN" "$API/deals?account=100231&from=2026-10-01T00:00:00Z&limit=100"
```
`/deals` is paged (oldest first, `limit` up to 1000). While `next_cursor` is not `null`, ask for the next page with `&cursor=<next_cursor>`.

### Place an order
```bash
# market buy 0.10 lot EURUSD with SL and TP
curl -X POST $API/orders -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{
  "account": "100231", "symbol": "EURUSD", "side": "buy", "type": "market",
  "qty": "10000", "sl": "1.08000", "tp": "1.09500", "client_order_id": "bot-20261008-001"
}'

# buy limit, good till cancelled
curl -X POST $API/orders -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{
  "symbol": "EURUSD", "side": "buy", "type": "limit", "qty": "100000",
  "limit_price": "1.08200", "client_order_id": "bot-20261008-002"
}'
```
`type` is `market` (default), `limit`, `stop` or `stop_limit`. Pending orders take `limit_price` / `stop_price`, and optionally `expire_at` (with `"tif": "gtd"`), `oco_group` and `trailing_distance`. Market orders take an optional `max_deviation_points` (slippage tolerance).

The answer is `201 {"order_id": "...", "client_order_id": "...", "account": "...", "replayed": false}`. The fill arrives asynchronously: follow it on the WebSocket, or poll `/orders`, `/positions` and `/deals`.

**Idempotency.** `client_order_id` (1-64 characters `A-Z a-z 0-9 . _ : -`, unique per account) makes retries safe. If a request times out, send it again with the same `client_order_id`: you get `200` with `"replayed": true` and the order that was already placed, never a second order. Reusing the id for a different order (another symbol or side) gets `409 client_order_id_conflict`. The `Idempotency-Key` header works the same way when the body has no `client_order_id`. Without either, every request is a new order.

### Modify and cancel orders
`{id}` is the `order_id` or your `client_order_id`.
```bash
curl -X PATCH $API/orders/bot-20261008-002 -H "Authorization: Bearer $TOKEN" -d '{"limit_price": "1.08150", "sl": "1.07500"}'
curl -X DELETE "$API/orders/bot-20261008-002?account=100231" -H "Authorization: Bearer $TOKEN"
```

### SL / TP and close positions
```bash
# set SL, remove TP; fields you leave out keep their value
curl -X PATCH $API/positions/5521 -H "Authorization: Bearer $TOKEN" -d '{"sl": "1.08300", "tp": null}'

# close half of a position (all of it without "qty"); idempotent like orders
curl -X POST $API/positions/5521/close -H "Authorization: Bearer $TOKEN" -d '{"qty": "5000", "client_order_id": "bot-close-7"}'
```

### Errors
Errors are JSON with an HTTP status:
```json
{ "error": { "code": "forbidden", "message": "read-only API key" } }
```

| Status | `code` | When |
|---|---|---|
| 400 | `bad_request`, `unknown_symbol` | invalid body or parameter, unknown symbol in an order, unsupported time in force |
| 401 | `unauthenticated` | no token, or an invalid or expired token |
| 403 | `forbidden` | a `read` key sends a command, or the account is not in your token |
| 404 | `unknown_order`, `unknown_position`, `unknown_account`, `unknown_symbol`, `no_quote` | the target does not exist (yet) |
| 409 | `client_order_id_conflict`, `duplicate_client_order_id` | the `client_order_id` is already used by another order |
| 422 | `insufficient_margin`, `order_rejected` | the risk check or the market rules rejected the command (see `message`) |
| 429 | `rate_limited` | too many requests or orders; retry after `Retry-After` seconds |
| 503 | `unavailable` | trading is temporarily unavailable |

## 4. WebSocket
`wss://trade.fxvps.ai/ws`: send the token in the first `auth` message, the same way the terminal does. The message types are documented in `crates/client-proto`.

A `read` token gets `forbidden: read-only API key` on any order command and on `PrefsSet`.

The account endpoints under `/api/client/*` accept the token for reading (`GET /api/client/me`, statements, copy-trading and IB overviews). Requests that move money or change links (`POST /api/client/funding`, KYC uploads, `ib/link`, `copy/subscribe`, `copy/unsubscribe`) answer `403 api_key_forbidden` for any API key.

## Limits
- Per-account order rate limit (same as the terminal).
- An API-key token always has only the `client` role. It cannot reach admin endpoints, cannot create or revoke keys, and cannot change passwords, 2FA, passkeys or sessions.
- Per-account order rate limit: one budget shared by the terminal, the WebSocket and the REST API (by default 10 orders per second, bursts up to 20).
- REST requests: 10 per second per API key, bursts up to 30 (by default). Each interactive login has its own budget.
- An API-key token always has only the `client` role. It cannot reach admin endpoints and cannot create new keys.
