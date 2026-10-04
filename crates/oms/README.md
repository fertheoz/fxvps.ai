# oms

Deterministic single-writer order management engine (`Engine`).

* Inputs are journaled `Envelope { seq, ts, cmd: Command }`; the engine never
  reads a clock. `Engine::replay(journal)` and `Engine::restore(snapshot)` +
  tail reproduce the exact `state_digest()`.
* Order lifecycle: New -> Accepted -> PartiallyFilled -> Filled, or
  Cancelled / Rejected / Expired (`OrderStatus::can_transition`).
* Order types: market, limit, stop, stop-limit; SL/TP; server-side trailing
  stop (activates once in profit by the distance, only ratchets); OCO groups;
  expiry; partial close; idempotent `client_order_id` per account.
* Position models: hedging (one position per order) and netting (one per
  symbol, reduce/flip with realized P&L).
* Routing per group: B-book fills at the marked-up client quote against the
  broker book; A-book sends `LpOrderRequest`s via the `LpRouter` trait, LP
  fills come back as `Command::LpFill` (deduplicated by exec id) and are
  allocated pro-rata (largest remainder) or FIFO to client orders. Optional
  aggregation of A-book orders into one LP order (`FlushLp`).
* Ledger postings: commission (client -> broker book), realized P&L
  (B-book: client <-> broker book; A-book: LP counterparty pays LP P&L,
  client gets P&L at marked-up prices, broker book keeps the markup), swaps
  on `Rollover`, negative balance protection after liquidation.
* Risk: pre-trade check on placement and on pending-order activation; margin
  call event and stop-out liquidation (largest loss first) on each quote.
* Invariants (`check_invariants`): ledger zero-sum, omnibus net LP position =
  sum of A-book client positions, no overfill.

Benchmark: `cargo bench -p oms --bench throughput` (B-book market open+close
with ledger postings, 100 accounts).

Known gaps: triggered pending A-book orders go to the LP as market orders (no
LP limit), LP partial fills are not rounded to the lot step, stop-out on
A-book waits for the LP fill before liquidating the next position, the
admin/state scans are O(positions) per account.
