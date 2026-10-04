# lp-simulator

LMAX-like FIX 4.4 **acceptor** for development and tests.

- Separate MD (`[md]`) and trading (`[trade]`) sessions with their own CompIDs/credentials.
- Random-walk prices per instrument (`initial_mid`, `tick_size`, `spread_ticks`,
  `volatility_ticks`, `depth`, `seed`), published as `MarketDataSnapshotFullRefresh (W)` every
  `tick_interval_ms` to subscribers; unknown SecurityID → MarketDataRequestReject (Y).
- Orders: market (IOC default, FOK), limit IOC/FOK/DAY/GTC; sweeps book levels producing
  partial fills (one ExecutionReport per level), IOC remainder canceled, FOK killed when
  liquidity is insufficient, DAY/GTC remainder rests and can be canceled (F) / replaced (G);
  rejects for unknown instrument, duplicate ClOrdID, bad qty/price.

```
cargo run -p lp-simulator -- services/lp-simulator/config/default.toml
```

Assumptions (unverified LMAX specifics): CompIDs, `SecurityIDSource=8`, ids other than
EUR/USD=4001, Username/Password in Logon, market orders only IOC/FOK.
Gaps: resting orders are not re-matched on price moves; fills do not deplete liquidity;
no incremental (X) feed yet.
