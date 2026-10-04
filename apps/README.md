# apps/

TypeScript uygulamaları (henüz boş — M1 kapsamı dışında). Plan (docs/03 §5):

- `web-terminal/` — React 19 + Vite trading terminali
- `backoffice/` — Next.js back office
- `desktop/` — Tauri 2 (web-terminal'i sarar)
- `mobile/` — Expo

Her uygulama pnpm workspace paketi olarak eklenir (`pnpm-workspace.yaml`), Turborepo ile derlenir.
