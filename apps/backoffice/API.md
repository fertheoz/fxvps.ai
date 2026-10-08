# Back-office API contract (`AdminApi`)

The UI talks to a single typed interface, `AdminApi` (`src/lib/api/types.ts`).
Two adapters implement it:

| Adapter | File | When |
|---|---|---|
| Mock (in-browser, seeded, deterministic) | `src/lib/api/mock.ts` | default; static demo / GitHub Pages / tests |
| HTTP | `src/lib/api/http.ts` | `NEXT_PUBLIC_API_URL` is set at build time |

The HTTP adapter talks to the admin API of `services/core-engine`
(`src/admin/`, axum). All endpoints below are implemented there; live updates
come from a server-sent events stream instead of polling.

### Running against a live core-engine

```sh
# terminal 1 (repo root): dev auth + demo data + CORS for next dev
CORE_DATA_DIR=/tmp/core CORE_DEV_AUTH=1 CORE_DEV_AUTH_ADMIN=1 CORE_SEED=1 \
CORE_CORS_ORIGINS=http://localhost:3000 cargo run -p core-engine
# terminal 2 (apps/backoffice)
NEXT_PUBLIC_API_URL=http://127.0.0.1:8090 pnpm dev
```

Without a token the UI shows a sign-in screen: paste a JWT from your IdP, or
(dev builds, or `NEXT_PUBLIC_DEV_AUTH=1`) pick a role and press *Get dev
token*, which calls `POST /auth/dev-token` (served only with
`CORE_DEV_AUTH=1` on a loopback `CORE_ADMIN_ADDR`, signed with a random
per-process key; `admin` only with `CORE_DEV_AUTH_ADMIN=1`). The token is kept
in memory + `sessionStorage` (this tab), never `localStorage`. The role shown in the UI comes from the token; the role
switcher exists only in mock mode. `pnpm e2e:live` builds the live export into
`out-live/`, starts core-engine (`cargo run`, or `$CORE_ENGINE_BIN`) with dev
auth and seeded data and runs `e2e-live/` against it.

## Conventions

- **Money**: every monetary field is an **integer in the currency's minor
  unit** (USD cents, JPY yen, BTC satoshi). JSON numbers must be safe integers
  (|n| ≤ 2^53−1). Floats are rejected (`MinorAmount` zod schema).
- Prices/lots are decimal numbers for display only; the core uses fixed-point.
- Timestamps: ISO-8601 UTC strings.
- Auth: bearer JWT (`Authorization: Bearer …`), claims `sub`, `name`, `role`
  (`admin|dealer|risk|support|readonly`) or the identity service `roles` array,
  `exp`, `amr`. Production: identity JWKS (`CORE_JWT_JWKS_URL`, RS256) with
  `CORE_JWT_ISSUER` / `CORE_JWT_AUDIENCE` checked and `CORE_REQUIRE_MFA=1`
  (mutating permissions need `amr` ∋ `otp|mfa|hwk`, else 403 `mfa_required`);
  also `CORE_JWT_JWKS_FILE`, `CORE_JWT_RS256_PUBLIC_KEY_FILE` or HS256
  (`CORE_JWT_HS256_SECRET`). Leeway 5 s. The server derives actor + role from the
  token and **re-checks RBAC** on every endpoint; client-side guards are UX
  only. CORS origins: `CORE_CORS_ORIGINS`.
- Errors: `4xx/5xx` with `{ "error": { "code": string, "message": string } }`.
  `403` = missing permission (`code: "forbidden"`, `permission` field).
- Mutations are appended to the immutable audit log server-side (admin
  journal `admin.jsonl`, with the actor from the token). Engine changes
  (deposits, withdrawals, group/symbol saves, force closes) are journaled as
  engine commands, so replay is deterministic.
- Request bodies are validated with the same zod schemas (`src/lib/schemas.ts`)
  on the client; the server must validate independently.

## Endpoints

| Method | Path | AdminApi | Permission | Body / notes |
|---|---|---|---|---|
| GET | `/v1/dashboard` | `dashboard()` | dashboard.view | `DashboardStats` |
| GET | `/v1/exposure` | `exposure()` | dashboard.view | `SymbolExposure[]` (client net, A/B split, LP hedge) |
| GET | `/v1/accounts?search=` | `listClients(q)` | clients.view | `Client[]` (masters + sub-accounts, `parentId`) |
| GET | `/v1/accounts/{id}` | `getClient(id)` | clients.view | `Client` or 404 |
| POST | `/v1/accounts/{id}/balance-ops` | `balanceOp(req)` | balance.deposit / .withdraw / .credit | `BalanceOpRequest`; header `Idempotency-Key` = `req.idempotencyKey` (required; same key + same request → same result with `replayed: true`, same key + different request → `409 idempotency_conflict`). Returns `{id, status: "applied" \| "pending_approval", newBalance, newCredit}`; amounts ≥ `settings.fourEyesThreshold` are always queued for a second approver. `422 rejected` when the engine refuses (e.g. withdrawal above free margin), `422 currency_mismatch` |
| GET | `/v1/approvals?status=pending_approval\|all` | `listApprovals(status)` | balance.approve or any balance.* | `ApprovalRequest[]` newest first |
| POST | `/v1/approvals/{id}/approve` | `approve(id)` | balance.approve | executes the op; `403 four_eyes` when the approver is the requester (same `sub`), `409 not_pending` when decided |
| POST | `/v1/approvals/{id}/reject` | `reject(id, reason)` | balance.approve | `{ reason }` |
| PATCH | `/v1/accounts/{id}/kyc` | `setKyc(id, kyc)` | clients.edit | `{ kyc: "none"\|"pending"\|"approved"\|"rejected" }` |
| GET | `/v1/groups` | `listGroups()` | groups.view | `Group[]` |
| PUT | `/v1/groups/{id}` | `saveGroup(g)` | groups.edit | `Group` (stopOut < marginCall) |
| POST | `/v1/groups/{id}/apply-preset` | `applyPreset(presetId, groupId)` | risk.edit | `{ presetId }` |
| GET | `/v1/symbols` | `listSymbols()` | symbols.view | `SymbolSpec[]` |
| PUT | `/v1/symbols/{name}` | `saveSymbol(s)` | symbols.edit | `SymbolSpec` |
| GET | `/v1/positions` | `listPositions()` | positions.view | `Position[]` |
| GET | `/v1/orders` | `listOrders()` | positions.view | `Order[]` |
| POST | `/v1/positions/force-close` | `forceClose(ids)` | positions.forceClose | `{ positionIds: string[] }` → `{ closed }` |
| GET | `/v1/risk/margin-calls` | `marginCalls()` | risk.view | `MarginCallRow[]` sorted by margin level asc |
| GET | `/v1/risk/presets` | `esmaPresets()` | risk.view | `EsmaPreset[]` |
| GET | `/v1/lp/sessions` | `listFixSessions()` | lp.view | `FixSession[]` (status, in/out seq, latency, rejects) |
| POST | `/v1/lp/sessions/{id}/reconnect` | `reconnect(id)` | lp.reconnect | sends Logon (35=A) |
| GET | `/v1/reports/trades` | `listTrades()` | reports.view | `Trade[]` |
| GET | `/v1/reports/statements` | `statements()` | reports.view | `Statement[]` |
| GET | `/v1/reports/lp-executions` | `listLpExecutions()` | reports.view | `LpExecution[]` newest first: LP order, fills (exec id, price), allocated client orders |
| GET | `/v1/reports/revenue` | `revenue()` | reports.view | `RevenueReport`: ledger legs per realized P&L / commission entry (client, broker, LP), totals all time and last 24 h |
| GET | `/v1/audit` | `listAudit()` | audit.view | `AuditEntry[]` newest first, append-only |
| GET | `/v1/admin-users` | `listUsers()` | users.view | `AdminUser[]` |
| PUT | `/v1/admin-users/{id}` | `saveUser(u)` | users.edit | `AdminUser` |
| GET | `/v1/settings` | `getSettings()` | settings.view | `Settings` |
| PUT | `/v1/settings` | `saveSettings(s)` | settings.edit | `Settings` |
| GET | `/v1/econ-calendar?from=&to=&currency=` | `listEconEvents(from, to)` | settings.view | `{from, to, events: EconEvent[]}` ordered by time; `from`/`to` epoch ms or ISO date (default last 7 days → 14 days ahead, at most 400 days); `currency` comma separated (`ALL` events always match) |
| POST | `/v1/econ-calendar` | `createEconEvent(e)` | settings.edit | `{time (epoch ms), currency, title, impact: low\|medium\|high, actual?, forecast?, previous?}`; `409 duplicate` for the same title + currency + time |
| PUT / DELETE | `/v1/econ-calendar/{id}` | `updateEconEvent` / `deleteEconEvent` | settings.edit | journaled (`calendar.event` / `calendar.delete` in the audit log) |
| POST | `/v1/econ-calendar/import` | `importEconWeek()` | settings.edit | fetches the ForexFactory weekly JSON server side (`CORE_ECON_FEED_URL` overrides); idempotent by (title, currency, time) → `{total, added, updated, unchanged, skipped}`; `502 feed_unavailable` when the feed is down / rate-limited |
| GET | `/v1/client/calendar?from=&to=&currency=` | terminal (`/api/client/calendar`) | client token | same shape as `/v1/econ-calendar`; read-only API keys allowed |
| GET | `/v1/me` | — | authenticated | `{sub, name, role, permissions}` |
| POST | `/v1/stream/ticket` | `subscribe()` | authenticated | `{ticket, expiresInMs}`: single-use, 30 s; the bearer token never goes into a URL |
| GET | `/v1/stream?ticket=` | `subscribe()` | ticket (or `Authorization` header) | SSE: `event: invalidate`, `data: {"topics": [AdminApi method names]}` (`"*"` = everything). Sent on admin mutations and whenever the engine sequence moves (client orders, fills, quotes). The UI invalidates those queries and stops polling while connected |
| POST | `/auth/dev-token` | — | none; **only with `CORE_DEV_AUTH=1`** (404 otherwise) | `{role, name?}` → `{token, expiresAt}` (random dev key, 8 h; `sub` = `dev-<role>`, chosen by the server; `admin` → 403 unless `CORE_DEV_AUTH_ADMIN=1`) |

Legacy engine routes (`/accounts`, `/commands`, ...) require the `admin`
role; `/health` is public.

## Roles

`admin`, `dealer`, `risk`, `support`, `readonly`. The full matrix lives in
`src/lib/rbac.ts` (`ROLE_PERMISSIONS`) and is rendered on the Users & roles
page. Highlights: dealers cannot move money; support cannot force-close or
issue credit; `risk` holds `balance.approve` (second approver); `readonly`
has only `*.view` (minus users/settings).

## Mapping notes (core-engine)

- Accounts are engine accounts (`id` = login); name/email/country are
  placeholders until a CRM is attached. `credit` is kept in the admin journal
  and added to equity for display; margin/stop-out use the engine figures.
- Groups map to `risk::GroupConfig` (id = name); `commissionType/Value` and
  `swapMultiplier` are not modelled by the engine and are echoed as defaults.
- Symbols map to `risk::SymbolSpec`; `tradeSessions`, `enabled` and `lp`
  are not modelled yet.

## Open items

- LP/FIX session list and reconnect (`/v1/lp/*`) return `[]`/404 until
  fix-gateway status is exposed to core-engine; closed-trade history
  (`/v1/reports/trades`) is not retained by the engine yet.
- Admin users are stored in the admin journal; login itself is the IdP's.
- Pagination/cursor params on list endpoints.
- Server-generated statements as PDF.
