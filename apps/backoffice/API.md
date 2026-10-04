# Back-office API contract (`AdminApi`)

The UI talks to a single typed interface, `AdminApi` (`src/lib/api/types.ts`).
Two adapters implement it:

| Adapter | File | When |
|---|---|---|
| Mock (in-browser, seeded, deterministic) | `src/lib/api/mock.ts` | default; static demo / GitHub Pages / tests |
| HTTP (stub) | `src/lib/api/http.ts` | `NEXT_PUBLIC_API_URL` is set at build time |

The HTTP adapter targets the future Rust `backoffice-api` service (axum, see
`docs/03-teknoloji-mimarisi.md` §4). Nothing in this document is implemented
server-side yet.

## Conventions

- **Money**: every monetary field is an **integer in the currency's minor
  unit** (USD cents, JPY yen, BTC satoshi). JSON numbers must be safe integers
  (|n| ≤ 2^53−1). Floats are rejected (`MinorAmount` zod schema).
- Prices/lots are decimal numbers for display only; the core uses fixed-point.
- Timestamps: ISO-8601 UTC strings.
- Auth: OIDC bearer token (`Authorization: Bearer …`), MFA enforced by the IdP.
  The server derives actor + role from the token and **re-checks RBAC**; the
  client-side guards are UX only.
- Errors: `4xx/5xx` with `{ "error": { "code": string, "message": string } }`.
  `403` = missing permission (`code: "forbidden"`, `permission` field).
- Mutations are appended to the immutable audit log server-side.
- Request bodies are validated with the same zod schemas (`src/lib/schemas.ts`)
  on the client; the server must validate independently.

## Endpoints

| Method | Path | AdminApi | Permission | Body / notes |
|---|---|---|---|---|
| GET | `/v1/dashboard` | `dashboard()` | dashboard.view | `DashboardStats` |
| GET | `/v1/exposure` | `exposure()` | dashboard.view | `SymbolExposure[]` (client net, A/B split, LP hedge) |
| GET | `/v1/accounts?search=` | `listClients(q)` | clients.view | `Client[]` (masters + sub-accounts, `parentId`) |
| GET | `/v1/accounts/{id}` | `getClient(id)` | clients.view | `Client` or 404 |
| POST | `/v1/accounts/{id}/balance-ops` | `balanceOp(req)` | balance.deposit / .withdraw / .credit | `BalanceOpRequest`; header `Idempotency-Key` = `req.idempotencyKey`. Returns `{status: "applied" \| "pending_approval"}`; amounts ≥ 4-eyes threshold from a user without `balance.approve` are queued |
| PATCH | `/v1/accounts/{id}/kyc` | `setKyc(id, kyc)` | clients.edit | `{ kyc: "none"\|"pending"\|"approved"\|"rejected" }` |
| GET | `/v1/groups` | `listGroups()` | groups.view | `Group[]` |
| PUT | `/v1/groups/{id}` | `saveGroup(g)` | groups.edit | `Group` (stopOut < marginCall) |
| POST | `/v1/groups/{id}/apply-preset` | `applyPreset(presetId, groupId)` | risk.edit | `{ presetId }` |
| GET | `/v1/symbols` | `listSymbols()` | symbols.view | `SymbolSpec[]` |
| PUT | `/v1/symbols/{name}` | `saveSymbol(s)` | symbols.edit | `SymbolSpec` |
| GET | `/v1/positions` | `listPositions()` | positions.view | `Position[]` (live; WS stream planned: `/v1/stream/positions`) |
| GET | `/v1/orders` | `listOrders()` | positions.view | `Order[]` |
| POST | `/v1/positions/force-close` | `forceClose(ids)` | positions.forceClose | `{ positionIds: string[] }` → `{ closed }` |
| GET | `/v1/risk/margin-calls` | `marginCalls()` | risk.view | `MarginCallRow[]` sorted by margin level asc |
| GET | `/v1/risk/presets` | `esmaPresets()` | risk.view | `EsmaPreset[]` |
| GET | `/v1/lp/sessions` | `listFixSessions()` | lp.view | `FixSession[]` (status, in/out seq, latency, rejects) |
| POST | `/v1/lp/sessions/{id}/reconnect` | `reconnect(id)` | lp.reconnect | sends Logon (35=A) |
| GET | `/v1/reports/trades` | `listTrades()` | reports.view | `Trade[]` |
| GET | `/v1/reports/statements` | `statements()` | reports.view | `Statement[]` |
| GET | `/v1/audit` | `listAudit()` | audit.view | `AuditEntry[]` newest first, append-only |
| GET | `/v1/admin-users` | `listUsers()` | users.view | `AdminUser[]` |
| PUT | `/v1/admin-users/{id}` | `saveUser(u)` | users.edit | `AdminUser` |
| GET | `/v1/settings` | `getSettings()` | settings.view | `Settings` |
| PUT | `/v1/settings` | `saveSettings(s)` | settings.edit | `Settings` |

## Roles

`admin`, `dealer`, `risk`, `support`, `readonly`. The full matrix lives in
`src/lib/rbac.ts` (`ROLE_PERMISSIONS`) and is rendered on the Users & roles
page. Highlights: dealers cannot move money; support cannot force-close or
issue credit; `risk` holds `balance.approve` (second approver); `readonly`
has only `*.view` (minus users/settings).

## Open items for the real backend

- 4-eyes approval queue endpoints (`GET/POST /v1/approvals`) — UI shows the
  pending state only.
- WebSocket push for positions/exposure/LP sessions instead of polling.
- Pagination/cursor params on list endpoints (the mock returns full lists).
- Server-generated statements as PDF.
