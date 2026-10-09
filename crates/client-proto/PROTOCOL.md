# fxvps.ai client protocol — v1 (current revision: v1.2)

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

## v1.2 additions (order types, protection, positions, history)

All additive: `Envelope.version` / `Hello.protocol_version` stay `1`. Servers older than
v1.2 answer the new requests with `Error{BAD_REQUEST, "unexpected message"}` and never
send the new fields; clients should treat absent fields as "not supported".

- **Account parameters.** `AuthOk.accounts` lists one `AccountInfo{account_id, currency,
  margin_mode, leverage, group}` per authorized account (same order as `account_ids`).
  `AccountSnapshot` also carries `margin_mode` and `leverage`. `margin_mode`:
  `NETTING` = at most one position per symbol (opposite fills reduce / reverse it);
  `HEDGING` = every fill opens its own position, several per symbol and side.
- **Order types.** `OrderType.STOP` (market order once the price reaches `stop_price`:
  buy stop when ask >= stop, sell stop when bid <= stop) and `STOP_LIMIT` (once the stop
  trades the order rests as a limit at `limit_price`; `OrderUpdate.stop_triggered`).
  `LIMIT`/`STOP_LIMIT` need `limit_price`, `STOP`/`STOP_LIMIT` need `stop_price`
  (else `BAD_REQUEST`). Triggers use the account group's (marked-up) prices.
- **Protection.** `PlaceOrder.sl` / `tp` / `trailing_distance` (price units, e.g.
  `0.0020` = 20 pips on EURUSD; rounded to whole points) are attached to the resulting
  position. SL must be below / TP above the entry for buys (mirrored for sells), else
  `ORDER_REJECTED`. The trailing stop is server side: once the price has moved
  `trailing_distance` into profit, the SL follows the closing price at that distance and
  never moves back. SL / TP / stop-out closes are market orders with server generated
  `client_request_id`s (`sl-…`, `tp-…`, `so-…`) and `close_position_id` set.
- **OCO.** `PlaceOrder.oco_group` (client chosen, per account, 0 = none): when one
  pending order of the group starts executing, the others are cancelled (`CANCELED`).
- **Expiry.** `PlaceOrder.expire_at_ns` (pending orders; `tif = GTD` requires it, GTC is
  the default without it) → `OrderUpdate{status: EXPIRED}` at that time.
  `FOK` is rejected; `IOC` / `GTD` only apply to market / pending orders respectively.
- **ModifyOrder** modifies a pending order **in place** (same `order_id`, same
  `target_request_id`): `qty`, `limit_price`, `stop_price`, `sl`, `tp`,
  `trailing_distance`, `expire_at_ns` / `clear_expiry`. Absent fields keep their value;
  with `replace_protection = true` the given `sl` / `tp` / `trailing_distance` replace
  the current ones and absent ones are removed. The full order is re-validated (margin,
  SL/TP side); a rejected modify leaves the order unchanged. Success: `Ack` +
  `OrderUpdate{status: NEW}` with the new parameters. (v1.1 servers implemented modify
  as cancel + re-place.)
- **OrderUpdate** now carries the order's full parameters: `order_type`, `qty`,
  `limit_price`, `stop_price`, `sl`, `tp`, `trailing_distance`, `oco_group`,
  `expire_at_ns`, `created_ns`, `stop_triggered`, `position_id` (position opened /
  increased) and `close_position_id` (position being closed).
- **Positions.** `Position.position_id` identifies a position; hedging accounts can hold
  several per symbol, so clients must key positions by `position_id` (fall back to
  `symbol` when it is empty, i.e. pre-v1.2 servers). New fields: `side`, `qty`
  (unsigned), `sl`, `tp`, `trailing_distance`, `open_time_ns`. `net_qty` is the signed
  quantity of *this* position; a `PositionUpdate` with `net_qty = 0` means the position
  is closed. A netting position that is reversed is closed (flat update) and a new
  position id is opened.
- **ModifyPosition**`{request_id, account_id, position_id, sl?, tp?, trailing_distance?}`
  sets the position's protection (full replacement: an absent field removes it).
  `Ack` + `PositionUpdate`; SL/TP on the wrong side of the current closing price →
  `ORDER_REJECTED`; unknown position → `UNKNOWN_ORDER`.
- **ClosePosition**`{request_id, account_id, position_id, qty?}` closes the whole
  position (qty absent) or part of it at market. `request_id` becomes the closing
  order's `client_request_id` (must be unique like a PlaceOrder's). More than the open
  (not already closing) quantity → `ORDER_REJECTED`.
- **OrderListRequest**`{request_id, account_id}` → `OrderList{orders}`: every working
  (pending) order in `OrderUpdate` shape.
- **Deals.** Each execution against a position is a `Deal{deal_id, order_id,
  client_request_id, position_id, symbol, side, entry IN|OUT, qty, price, realized_pnl,
  commission, ts_ns, reason CLIENT|STOP_LOSS|TAKE_PROFIT|STOP_OUT}` (account currency;
  commission negative = cost; the fill's commission is booked on its first deal). A
  netting reversal yields an `OUT` and an `IN` deal. New deals are pushed as
  `DealUpdate{account_id, deal}`.
- **DealHistoryRequest**`{request_id, account_id, from_ns?, to_ns?, limit, cursor}` →
  `DealHistory{deals, next_cursor}`: oldest first, at most `limit` (default 500, max
  5000). Repeat with `cursor = next_cursor` until it is empty. History comes from the
  engine's journaled state, so it survives restarts.
- `OrderList` / `DealHistory` for an account not in the token → `FORBIDDEN`.

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

### v1.3

- `Position.lp_tp` / `Position.lp_sl` (12, 13) and `OrderUpdate.lp_resting` (27): the level / entry rests at the liquidity provider as a real order (filled by the LP, never triggered on the broker quote). Absent = false on older servers.
