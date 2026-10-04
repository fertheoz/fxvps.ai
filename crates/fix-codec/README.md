# fix-codec

Minimal FIX 4.4 tag=value codec.

- `frame_len` — stream framing; validates `8=`, `9=` BodyLength, `10=` CheckSum and a 1 MiB cap.
- `RawMessage` / `FieldView` — borrowed (zero-copy) field access and repeating-group splitting.
- `Header` + `Body` — typed messages: Logon A, Heartbeat 0, TestRequest 1, ResendRequest 2,
  Reject 3, SequenceReset 4, Logout 5, MarketDataRequest V, MarketDataSnapshotFullRefresh W,
  MarketDataIncrementalRefresh X, NewOrderSingle D, ExecutionReport 8, OrderCancelRequest F,
  OrderCancelReplaceRequest G, OrderCancelReject 9. Anything else decodes to `Body::Unknown`.
- Prices/quantities are `domain::Fixed` (i64, 8 decimals). Values with more precision are
  rejected instead of rounded; no `f64` anywhere.

## Why an own codec instead of hotfix / quickfix-rs

docs/03 lists HotFIX (primary) and quickfix-rs (fallback). For M1 we chose a small own codec:

| | own codec | hotfix | quickfix-rs |
|---|---|---|---|
| Acceptor support (needed for `lp-simulator`) | yes | **no** (initiator only) | yes |
| Build deps | pure Rust | pure Rust | C++ toolchain + QuickFIX build |
| Fixed-point money | native `Fixed` | string/decimal mapping needed | `f64`/string getters |
| Control over hot path / allocation | full | partial | none (FFI) |
| Maturity | new, covered by unit + proptest | small community (docs/03) | QuickFIX C++ is mature |

The subset FIX actually used against a single LP (≈15 message types) is small, the session
layer is where the risk lives, and we need an acceptor for the simulator anyway. The codec is
isolated behind `encode`/`decode`, so swapping in quickfix-rs for conformance comparison
(planned spike in docs/03 §risks) remains possible. A data-dictionary driven validator and
`cargo-fuzz` target are follow-ups.

LMAX specifics (`SecurityIDSource(22)=8`, Username 553 / Password 554 in Logon) are
**assumptions** pending the LMAX FIX spec — see docs/02 `[DOĞRULA]`.
