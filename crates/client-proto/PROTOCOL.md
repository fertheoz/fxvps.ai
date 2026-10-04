# fxvps.ai client protocol — v1

Schema: [`proto/fxvps_client_v1.proto`](proto/fxvps_client_v1.proto) (package `fxvps.client.v1`).
Rust types are generated at build time with `prost-build` using the vendored `protoc`
(`protoc-bin-vendored`), so no system `protoc` is required.

## Transport and encoding

- WebSocket at `GET /ws` (client-gateway). Each frame carries exactly one `Envelope`.
- **Binary frames = protobuf** (production). **Text frames = JSON** (debug): same schema,
  proto field names in `snake_case`, oneof as `{"body": {"<variant>": {...}}}`, enums as
  integers, missing fields take proto3 defaults. Example:
  `{"version":1,"body":{"hello":{"protocol_version":1}}}`.
- The encoding of a connection is fixed by the client's first frame; the server answers in
  the same encoding.
- `Envelope.version` = sender's protocol version; `Envelope.seq` = per-sender frame counter
  (informational, for gap diagnostics).
- Max inbound frame: 64 KiB (configurable).

## Numbers

All prices and quantities are `Decimal { int64 value; uint32 scale }`, real value
`value * 10^-scale`. The server emits `scale = 8` (domain `Fixed`). Clients may send any
scale; values not representable with 8 decimals are rejected. No floating point.
Timestamps are `uint64` nanoseconds since the UNIX epoch. Symbols are compact (`EURUSD`).

## Session flow

1. Client → `Hello{protocol_version=1, client_name, max_quote_hz}`.
   Server → `Hello{protocol_version, max_quote_hz=<effective>}` (the effective rate is the
   client's request capped by the server maximum; 0 = server default).
   Unsupported version → `Error{UNSUPPORTED_VERSION}` + close 4002.
2. Client → `Auth{token}` (JWT bearer). Server → `AuthOk{subject, account_ids, expires_at_s}`
   then one `AccountSnapshot` per authorized account. Invalid token or any other message →
   `Error{UNAUTHENTICATED}` + close 4001. Hello+Auth must complete within the auth timeout
   (default 5 s) or the server closes with 4003.
3. Normal operation (any order):
   - `Subscribe{request_id, symbols}` / `Unsubscribe` → `Ack{request_id}` or
     `Error{UNKNOWN_SYMBOL}`. Subscriptions are per connection and additive.
   - `QuoteBatch` (server): conflated top-of-book, **at most one quote per symbol per batch,
     latest wins**, at most `max_quote_hz` batches per second. Intermediate ticks are
     intentionally dropped; clients must treat every quote as a full replacement.
   - `CandleRequest{symbol, timeframe M1..D1, from_ns, to_ns, limit}` →
     `CandleResponse` (mid-price OHLC, oldest first, at most `limit`, default 500).
   - `SymbolListRequest{request_id}` → `SymbolList{request_id, instruments}`: every symbol
     the gateway serves with `base`, `quote`, `tick_size`, `digits`, `qty_step` and
     `contract_size` (base units per lot). Quantities on the wire are always base units.
     Servers predating this message answer `Error{BAD_REQUEST}`; clients should fall
     back to local defaults.
   - `PlaceOrder{request_id, account_id, symbol, side, order_type, qty, limit_price, tif}`,
     `CancelOrder{request_id, account_id, target_request_id}`,
     `ModifyOrder{request_id, account_id, target_request_id, qty?, limit_price?}`:
     `Ack{request_id}` means accepted for routing; failures return
     `Error{request_id, code}` (FORBIDDEN, RATE_LIMITED, UNKNOWN_SYMBOL, UNKNOWN_ORDER,
     BAD_REQUEST, UNAVAILABLE, and since v1.1 INSUFFICIENT_MARGIN when the pre-trade margin
     check fails or ORDER_REJECTED for other risk rejections). `request_id` of a PlaceOrder must be unique per account and
     identifies the order for its whole life (cancel/modify target it).
     Market orders default to IOC; limit orders to GTC.
   - `OrderUpdate` (server): every execution of an order of an authorized account, with
     `client_request_id` = originating PlaceOrder `request_id`. Sent to every connection
     authorized for the account.
   - `PositionUpdate` (server): net position after each fill and (throttled) as prices
     move; `unrealized_pnl` (v1.1) is in account currency at the account group's prices.
   - `AccountSnapshot` (server, v1.1): also pushed whenever balance / equity / margin change
     (`free_margin`, `margin_level` percent). Quotes are the account group's marked-up prices.
   - `Ping{nonce}` → `Pong{nonce}`. Server sends `Heartbeat` every 15 s.

## Authorization

JWT claims: `sub`, `exp` (required), `accounts: [string]`. A connection may only trade and
receive account events for accounts listed in `accounts`. Keys: HS256 shared secret, RS256
PEM, or a JWKS (selected by `kid`). The built-in HS256 key is for local development only.

## Flow control

- Order commands are rate limited per account (token bucket; default 10/s, burst 20) →
  `Error{RATE_LIMITED}`.
- Each connection has a bounded outbound queue. If it fills (client not reading fast
  enough), the server closes the connection with **4008 (slow consumer)** instead of
  buffering. Reconnect and resubscribe; account state is resent as `AccountSnapshot`.

## Close codes

| Code | Meaning |
|------|---------|
| 1000 | Normal / server shutdown |
| 4001 | Unauthenticated |
| 4002 | Protocol error (bad frame, unexpected first message, unsupported version) |
| 4003 | Hello/Auth timeout |
| 4008 | Slow consumer |

## Compatibility

Field numbers are frozen. New fields and messages may be added in v1; clients must ignore
unknown fields and unknown `body` variants. Breaking changes bump the protocol version.
