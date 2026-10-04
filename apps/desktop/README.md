# @fxvps/desktop

Tauri 2 desktop shell around the `apps/terminal` web build.

- `build.frontendDist` = `../../terminal/dist` (built by `beforeBuildCommand`)
- `build.devUrl` = `http://localhost:5173` (terminal Vite dev server, started by `beforeDevCommand`)
- Rust crate: `src-tauri/` — **excluded** from the root Cargo workspace because it
  needs GTK/WebKit system libraries on Linux; built and linted by
  `.github/workflows/desktop.yml` (fmt + clippy `-D warnings` + tests on Ubuntu,
  then unsigned `tauri build --debug --no-bundle` on ubuntu/windows/macos).

## Native features

| Feature | Where |
| --- | --- |
| Detach chart into its own window | command `open_chart_window { symbol, timeframe? }` → window `chart-<SYMBOL>` loading `index.html?view=chart&symbol=..&tf=..` |
| Tray with connection status | `src/tray.rs`; command `set_connection_status { status, latencyMs? }` (`connecting|connected|reconnecting|disconnected`) |
| Native notifications (fills, price alerts) | command `notify_event { event: { kind: "fill", symbol, side, lots, price } \| { kind: "price_alert", symbol, price, message? } }` |
| Global shortcut | `CommandOrControl+Shift+F` shows/focuses the terminal |
| Single instance | second launch focuses the running window |
| Window state persistence | `tauri-plugin-window-state` (size, position, maximized; also for chart windows) |
| Secure credentials | commands `credential_set/get/delete { account, secret? }` → OS keychain via the `keyring` crate (Keychain / Credential Manager / Secret Service). Secrets are never written to disk or logged. |
| Auto-updater | `tauri-plugin-updater`, config only (see below) |

Calling from the terminal (follow-up, terminal is unchanged in M5):

```ts
import { invoke } from '@tauri-apps/api/core';
await invoke('open_chart_window', { symbol: 'EURUSD', timeframe: 'H1' });
await invoke('set_connection_status', { status: 'connected', latencyMs: 12 });
await invoke('notify_event', { event: { kind: 'fill', symbol: 'EURUSD', side: 'buy', lots: '0.10', price: '1.08506' } });
```

## Security model (Tauri 2 ACL)

- `src-tauri/permissions/app.toml` defines `allow-app-commands` and `allow-chart-commands`.
- `capabilities/main.json` (window `main`): core defaults, notifications, window state, updater, app commands.
- `capabilities/chart-windows.json` (windows `chart-*`): core defaults, window state, `notify_event` only —
  detached charts can't read credentials or trigger updates.
- No fs / shell / http plugins. A CSP is set in `tauri.conf.json`.
- `open_chart_window` validates the symbol (`[A-Z0-9._-]{1,32}`) before using it in a label/URL.

## Auto-updater: keys and signing (nothing committed)

`plugins.updater.pubkey` is the placeholder `REPLACE_WITH_TAURI_UPDATER_PUBLIC_KEY` and
`bundle.createUpdaterArtifacts` is `false`, so CI builds need no secrets. To ship updates:

1. `pnpm tauri signer generate -w ~/.tauri/fxvps-updater.key` (keep the private key offline / in a secret manager).
2. Put the **public** key into `plugins.updater.pubkey`, set `bundle.createUpdaterArtifacts: true`.
3. In the release workflow, provide repo secrets `TAURI_SIGNING_PRIVATE_KEY` and
   `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` as env for `tauri-action`, plus OS code-signing
   secrets (Apple: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
   `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`; Windows: certificate thumbprint in
   `bundle.windows.certificateThumbprint` or Azure Trusted Signing).
4. Serve `latest.json` from the endpoint in `plugins.updater.endpoints`.

## Local development

```
# Linux: sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libdbus-1-dev
(cd ../terminal && pnpm install --ignore-workspace --frozen-lockfile)
pnpm install --ignore-workspace --frozen-lockfile
pnpm dev                 # terminal dev server + native window
pnpm build:debug         # unsigned debug binary, no installers
(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test)
```

## Follow-ups

- Terminal: handle `?view=chart&symbol=..&tf=..` (single-chart layout) and call the
  commands above when running inside Tauri (`'__TAURI_INTERNALS__' in window`).
- Release workflow with signing + updater artifacts once keys exist.
