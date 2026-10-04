# Terminal ⇄ Gateway WebSocket protocol (v1, draft)

Status: **draft** for the M3 web terminal. Implemented client-side by
`src/api/ws.ts` (`WsTradingApi`); the server side (price-edge + client-api,
see `docs/03-teknoloji-mimarisi.md`) does not exist yet. JSON text frames are
used for v1 for debuggability; the hot path (quotes/depth) is expected to move
to protobuf/SBE binary frames later without changing the message semantics.

Select the adapter in the browser with `?api=ws&url=wss://gateway.example/ws`.

## Conventions

| Item | Rule |
|---|---|
| Transport | One WebSocket per browser (later: shared via SharedWorker across tabs). |
| Encoding | UTF-8 JSON, one message per frame. |
| Money | Integer **minor units** of the account currency (`balance: 10000000` = 100,000.00 USD). Never floats. |
| Volume | Integer **centi-lots** (`10` = 0.10 lot). |
| Prices | JSON numbers already rounded to the symbol's `digits`. Servers MUST NOT send more decimals than `digits`. |
| Time | Unix **milliseconds** (`time`, `openTime`, `expiry`), except bar `time` which is Unix **seconds** (bar open, UTC). |
| Ids | Strings. Orders carry a client-generated `clientId` (UUID) for idempotent retries. |

Types referenced below (`Account`, `SymbolSpec`, `Quote`, `Bar`, `Depth`,
`Position`, `PendingOrder`, `Deal`, `OrderRequest`, `OrderResult`,
`TradingEvent`) are defined in `src/api/types.ts` — that file is the schema.

## Client → server

### request
```json
{ "type": "request", "id": 7, "method": "order.place", "params": { ... } }
```
`id` is a per-connection increasing integer; the server answers with a
`response` carrying the same `id`.

| method | params | result |
|---|---|---|
| `hello` | `{ protocol: 1, token?: string }` | `{ serverTime: number, sessionId: string }` — MUST be first. `token` is the OIDC access token. Tokens are never put in the URL. |
| `accounts.list` | `{}` | `Account[]` (the user's master account and its sub-accounts; `parentId` links sub-accounts) |
| `symbols.list` | `{}` | `SymbolSpec[]` |
| `bars.get` | `{ symbol, timeframe: "M1"…"MN", count }` | `Bar[]` ascending by time |
| `history.deals` | `{ accountId, from?, to? }` | `Deal[]` |
| `order.place` | `OrderRequest` | `OrderResult` |
| `order.modify` | `{ accountId, orderId, price?, limitPrice?, sl?, tp?, expiry? }` | `OrderResult` |
| `order.cancel` | `{ accountId, orderId }` | `OrderResult` |
| `position.modify` | `{ accountId, positionId, sl?, tp? }` (omitted = remove) | `OrderResult` |
| `position.close` | `{ accountId, positionId, volume? }` (omitted = full close) | `OrderResult` |

### subscribe / unsubscribe
```json
{ "type": "subscribe",   "channel": "quotes", "symbols": ["EURUSD", "XAUUSD"] }
{ "type": "unsubscribe", "channel": "depth",  "symbols": ["EURUSD"] }
```
Channels: `quotes`, `depth`. Account events (`positions`, `orders`, `deal`,
`account`, `journal`) are pushed automatically for every account the session
may see after `hello`. Subscriptions are re-sent by the client after reconnect.

### ping
```json
{ "type": "ping", "t": 1759540000000 }
```
Sent every 5 s; the server echoes `t` in a `pong`. The client shows the
round-trip as the latency badge.

## Server → client

```json
{ "type": "response", "id": 7, "ok": true,  "result": { "ok": true, "positionId": "1002", "price": 1.08499 } }
{ "type": "response", "id": 8, "ok": false, "error": { "code": "NOT_ENOUGH_MONEY", "message": "..." } }
{ "type": "quotes", "quotes": [ { "symbol": "EURUSD", "bid": 1.0848, "ask": 1.08486, "time": 1759540000123, "dayOpen": 1.0835, "dayHigh": 1.0861, "dayLow": 1.0829 } ] }
{ "type": "depth",  "depth": { "symbol": "EURUSD", "bids": [{ "price": 1.0848, "volume": 650 }], "asks": [...], "time": 1759540000123 } }
{ "type": "event",  "event": { "type": "positions", "accountId": "100001", "positions": [ ... ] } }
{ "type": "pong",   "t": 1759540000000 }
```

- `quotes` frames are **conflated** server-side (latest per symbol, ≤ 20 Hz per
  symbol per connection); the client additionally batches them to one render
  per animation frame.
- `depth` is a full snapshot (top N levels, N ≤ 20). Incremental depth is a v2 item.
- `event.positions` / `event.orders` are full snapshots per account (simple
  and idempotent; deltas can be added later behind a capability flag).
- `event.deal` is append-only. `event.account` carries the new balance after
  realized P/L. `event.connection` is client-local and never sent by the server.
- Business rejections (validation, margin) arrive as `response.ok: true` with
  `result.ok: false` so the UI can show the reason; transport/auth errors use
  `response.ok: false`.

### Error codes (draft)
`UNAUTHENTICATED`, `FORBIDDEN`, `BAD_REQUEST`, `UNKNOWN_SYMBOL`, `MARKET_CLOSED`,
`INVALID_VOLUME`, `INVALID_PRICE`, `INVALID_STOPS`, `NOT_ENOUGH_MONEY`,
`TRADE_DISABLED`, `RATE_LIMITED`, `INTERNAL`.

## Reconnect

On close the client emits `reconnecting`, waits `min(30s, 500ms·2^n)`, opens a
new socket, sends `hello`, re-subscribes and expects fresh `positions`/`orders`
snapshots. In-flight requests are rejected with `Connection closed`; order
placement is safe to retry with the same `clientId`.
