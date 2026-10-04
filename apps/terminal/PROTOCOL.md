# Terminal ⇄ gateway protocol

The terminal speaks the client gateway protocol defined in
[`crates/client-proto/PROTOCOL.md`](../../crates/client-proto/PROTOCOL.md)
(schema: [`crates/client-proto/proto/fxvps_client_v1.proto`](../../crates/client-proto/proto/fxvps_client_v1.proto)),
using binary protobuf frames.

- TS types are generated into `src/api/gen/` with `pnpm proto:gen` (buf + protoc-gen-es); the
  generated code is committed. Re-run after changing the `.proto`.
- Adapter: `src/api/ws.ts` (`WsTradingApi`). Unit mapping: volume in centi-lots ⇄ base units
  (`contract_size` from `SymbolList`), money minor units ⇄ `Decimal` major units, prices ⇄
  `Decimal` via big.js (`src/api/decimal.ts`), never floats on the wire.
- Dev login: `?api=ws&url=ws://localhost:8080/ws&token=<jwt>` or the **Gateway** button in the
  top bar. The choice is stored in sessionStorage (`?api=mock` resets it).
