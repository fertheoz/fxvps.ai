# ledger

Double-entry, append-only, event-sourced ledger.

* Hierarchy: `LpOmnibus` -> `BrokerBook` -> `Client` sub-accounts; `External`
  accounts (bank/PSP, LP counterparty) are outside the tree.
* Balances per (account, currency) in `money::Money` minor units.
* `TxnKind`: deposit, withdrawal, adjustment, realized P&L, commission, swap,
  negative-balance protection, transfer.
* Invariants: every transaction sums to zero per currency (enforced on post
  and on replay); withdrawals/transfers cannot overdraw a client.
* Idempotency keys (FNV fingerprint of kind+postings): same key + same content
  = `Duplicate`, same key + different content = `IdempotencyConflict`.
* `snapshot()` / `restore(snapshot, tail)` / `replay(events)` all produce the
  identical `state_digest()`; proptests cover random op sequences.
