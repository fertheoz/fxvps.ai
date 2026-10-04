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
* **Admin HTTP (axum)**: `GET /health`, `GET /accounts`, `GET /accounts/{id}`,
  `GET /accounts/{id}/positions`, `POST /commands` (JSON `oms::Command`).
  No auth yet: binds `127.0.0.1:8090` by default (`CORE_ADMIN_ADDR`).

Run: `CORE_DATA_DIR=./data cargo run -p core-engine`.

Gaps: no real LP routing (A-book orders are filled at the current quote by
the simulator loop; wiring to M1's fix-gateway is next), journal is fsync'd
only at snapshot time, no journal compaction, no auth on the admin API.
