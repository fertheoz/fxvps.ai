# core-engine

Service wrapper around `oms::Engine`:

* **Single writer**: one OS thread owns all OMS/risk/ledger state; callers
  send `Request`s over a bounded `std::sync::mpsc::sync_channel`
  (`EngineHandle`, async and blocking variants).
* **Journal**: each command is stamped (seq, ns timestamp), appended to
  `journal.jsonl`, then applied. Simulated LP fills are journaled as normal
  commands, so replay never needs the LP.
* **Snapshots** every `CORE_SNAPSHOT_EVERY` commands (default 1000) and on
  shutdown (`snapshot.json`, written atomically). Startup = snapshot + journal
  tail; a torn last line is ignored.
* **Back-office admin API** (`src/admin/`, contract in
  `apps/backoffice/API.md`): `/v1/*` with JWT bearer auth and server-side
  RBAC (roles `admin`/`dealer`/`risk`/`support`/`readonly`, same matrix as
  the back office), balance operations with `Idempotency-Key` and 4-eyes
  approval (amounts >= `fourEyesThreshold` are queued and must be approved
  by a *different* user with `balance.approve`), audit log of every mutation
  with its actor, groups/symbols CRUD as journaled `SetGroup`/`AddSymbol`
  engine commands, exposure, margin-call queue, force close, and a
  server-sent-events stream (`GET /v1/stream`) of invalidation hints.
  Admin-only state lives in `admin.jsonl` (append-only, fsync'd, replayed on
  start).
* **Legacy engine routes**: `GET /accounts`, `GET /accounts/{id}`,
  `GET /accounts/{id}/positions`, `POST /commands` (raw `oms::Command`) now
  require an `admin` token; `GET /health` is open. Binds `127.0.0.1:8090` by
  default (`CORE_ADMIN_ADDR`).

| Env | Meaning |
|---|---|
| `CORE_JWT_RS256_PUBLIC_KEY_FILE` | PEM public key of the IdP (RS256) |
| `CORE_JWT_HS256_SECRET` | HS256 shared secret |
| `CORE_DEV_AUTH=1` | enables `POST /auth/dev-token {role, sub?, name?}`; without a secret uses a public DEV key (never in production) |
| `CORE_CORS_ORIGINS` | comma separated allowed origins (`*` = any); unset = no CORS |
| `CORE_SEED=1` | seeds demo symbols/groups/accounts/positions into an empty engine |

Startup fails without a JWT key unless `CORE_DEV_AUTH=1`.

Run (dev): `CORE_DATA_DIR=./data CORE_DEV_AUTH=1 CORE_SEED=1 CORE_CORS_ORIGINS=http://localhost:3000 cargo run -p core-engine`.

Gaps: no real LP routing (A-book orders are filled at the current quote by
the simulator loop; wiring to M1's fix-gateway is next), journal is fsync'd
only at snapshot time, no journal compaction.
