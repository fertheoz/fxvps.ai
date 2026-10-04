# @fxvps/mobile

Expo SDK 57 + React Native 0.86 + expo-router, TypeScript strict.

Screens: Watchlist (tab), Positions (tab), Account + settings (tab), Chart
(`/chart/[symbol]`, SVG candlesticks via `react-native-svg`, live last bar),
Trade ticket (`/trade/[symbol]`, modal). Dark/light/system theme and EN/TR
(device locale default, persisted in AsyncStorage).

Trading logic (money math, validation, `TradingApi` types and `MockTradingApi`)
comes from `packages/trading-core`, consumed by **source path** — no workspace
install needed:

- TypeScript: `paths` in `tsconfig.json`
- Metro: `metro.config.js` (`watchFolders` + `resolveRequest`)
- Jest: `moduleNameMapper` + `modulePaths` in `jest.config.js`

Third-party imports of trading-core (`big.js`) resolve from this app's
`node_modules`, so `big.js` is a direct dependency here too.

## Commands

```
pnpm install --ignore-workspace --frozen-lockfile   # own lockfile, hoisted (.npmrc)
pnpm typecheck && pnpm lint && pnpm test
pnpm export:web     # expo export --platform web -> dist/
pnpm start          # Expo dev server (Expo Go / simulators)
```

The app runs on the in-process mock backend. Wiring a real gateway means
passing a different `TradingApi` to `<TradingProvider api={...}>`.

Gaps: no native (EAS) builds in CI yet; no app icons/branding beyond Expo
defaults; no push notifications.
