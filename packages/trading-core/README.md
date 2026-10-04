# @fxvps/trading-core

Pure TypeScript trading logic (no React, no DOM requirement): money math (big.js,
minor units / centi-lots), order ticket validation, `TradingApi` types, mock symbols
and the in-process `MockTradingApi`, bar/depth/indicator helpers.

Copied verbatim from `apps/terminal/src/{api,lib}` in M5. **Follow-up:** make
`apps/terminal` import from this package and delete its copies (kept untouched in
M5 so the terminal PR surface stays zero).

```
pnpm install --ignore-workspace --frozen-lockfile
pnpm typecheck && pnpm lint && pnpm test
```
