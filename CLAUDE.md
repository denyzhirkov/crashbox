# `crashbox` — Project Rules

Tiny self-hosted, Sentry-compatible error tracking server with a lightweight UI. Single-binary Rust backend (Axum + SQLx + SQLite) serving an embedded SolidJS frontend, packaged as one Docker container. Project-specific conventions; generic engineering principles live in `~/.claude/CLAUDE.md`. Source-of-truth for scope and roadmap — `crashbox_master_prompt.md`.

---

## Layout

```
crashbox/
├─ backend/
│  ├─ Cargo.toml
│  ├─ migrations/             # SQLx migrations, SQLite-first
│  └─ src/
│     ├─ main.rs              # bin entry, wiring only
│     ├─ config.rs            # CRASHBOX_* env → validated Config
│     ├─ app_state.rs         # AppState (db pool, config, services)
│     ├─ http/                # Axum routes — thin handlers
│     │  ├─ routes.rs  auth.rs  ingest.rs  projects.rs  issues.rs  health.rs
│     ├─ sentry/              # protocol layer — envelope, dsn, normalize, grouping
│     ├─ db/                  # SQLx repositories per aggregate
│     ├─ security/            # password hashing, sessions
│     └─ jobs/                # background cleanup / retention
├─ frontend/                  # SolidJS + Vite + TailwindCSS
│  └─ src/  api/ pages/ components/
├─ Dockerfile  docker-compose.yml
├─ docs/  protocol.md  configuration.md  development.md
```

- `http/*` handlers are **thin wrappers**. No business logic — only request decode, call into the right module, serialize the response.
- Protocol/domain logic (envelope parsing, normalization, grouping, dsn) lives in `sentry/`. HTTP and DB layers depend on it, never the other way around.
- DB access goes through `db/*` repositories. SQL doesn't leak into handlers.
- Frontend is **embedded** into the backend binary at build time (e.g. `rust-embed`) — one binary serves API + static assets.

---

## Project conventions

- **Domain → service → adapters.** `sentry/` is the domain (envelope, event, grouping); `db/` and `http/` are adapters. Handlers never call SQLx directly through skip layers; routes never reach into the database module without going through a typed repository call.
- **One module, one responsibility.** Envelope parsing in `sentry/envelope.rs`, normalization in `sentry/normalize.rs`, grouping in `sentry/grouping.rs`. Don't grow `ingest.rs` into a god-handler.
- **Errors:** `thiserror` for typed domain/db errors; convert to HTTP at the handler boundary via a single `AppError` → `IntoResponse` impl. `anyhow` only in `main.rs` startup glue.
- **Config is centralized.** `config::Config` is the single validated entry point — read all `CRASHBOX_*` env vars on startup, fail loud if anything required is missing or malformed. No scattered `std::env::var` calls.
- **No `unwrap()` / `expect()` outside `main.rs`, tests, or migrations.** Ingestion path *must not panic* on malformed input — that's an acceptance criterion.
- **Money/IDs/time:** UUID or ULID for external IDs; timestamps stored as ISO-8601 text (per schema), always UTC; convert at the edge.

## Domain & Architecture

- **Ingestion is the hot path.** `POST /api/:project_id/envelope[/]` must accept both trailing-slash variants, enforce `CRASHBOX_MAX_ENVELOPE_BYTES` *before* allocating the full body, and return 2xx quickly. Heavy work (grouping, indexing) happens after the body is decoded but stays synchronous and bounded for MVP — no background queue.
- **Envelope parser is line-oriented and resilient.** Unknown item types are skipped (or stored raw, per `CRASHBOX_STORE_RAW_UNSUPPORTED_ITEMS`); a bad item does not poison the rest of the envelope. Only return 400 if the envelope is unparseable at the framing level.
- **Always preserve `raw_json`.** Normalization extracts fields for indexing/UI, but the original event JSON is the source of truth and is stored verbatim in `events.raw_json`.
- **Grouping is deterministic and documented.** `sentry/grouping.rs::fingerprint()` is the single entry point. Don't scatter fingerprint logic across handlers. Custom event `fingerprint` field wins; else exception-based; else message-based; else fallback. We do **not** try to bit-match Sentry — document the difference.
- **DSN format:** `http(s)://<public_key>@host[:port]/<project_id>`. SDK derives the endpoint as `/api/<project_id>/envelope/`. `sentry/dsn.rs` owns parse + format; nothing else builds DSNs by string concat.
- **Bootstrap is idempotent.** Admin user / default project are created only when none exist. Never overwrite an existing password unless `CRASHBOX_FORCE_ADMIN_RESET=true`. Generated public/secret keys are logged once at startup; secret values are not re-logged.

## Performance & Resilience

- **Single container, low idle footprint.** No background workers beyond the retention job. No Kafka, ClickHouse, Redis. SQLite + the Rust process is the whole runtime.
- **Bounded everything.** Body size (`CRASHBOX_MAX_ENVELOPE_BYTES`, `CRASHBOX_MAX_EVENT_BYTES`), per-project rate limit (`CRASHBOX_MAX_EVENTS_PER_MINUTE_PER_PROJECT`), max events per issue, retention days. Every limit is enforced — no unbounded loops, no unbounded allocations.
- **Graceful degradation:**
  - Malformed envelope → 400 with a short reason, no panic.
  - Unknown item type → ignore (or store raw) per config, keep processing the rest.
  - DB write failure on one event → log structured error, return 5xx for that request only; do not corrupt the issue counter.
  - Migration mismatch on startup → fail loud, refuse to serve.
- **Retention runs on `CRASHBOX_CLEANUP_INTERVAL_SECONDS`.** It deletes old events beyond `CRASHBOX_RETENTION_DAYS`, but keeps the last N per issue (`CRASHBOX_MAX_EVENTS_PER_ISSUE`) and preserves issue summaries longer than raw events.

## Errors & Logs

- Logging — `tracing` + `tracing-subscriber`. Span every ingestion request with `project_id`, `event_id`, envelope size; structured fields, no ad-hoc `println!`.
- `CRASHBOX_LOG_LEVEL` controls the level. Default `info`. Ingestion debug only on demand.
- **Never log secrets.** No plaintext passwords, no full DSN (mask the public key tail in regular logs; the one-time bootstrap log is the only exception). No raw `Authorization` headers.
- HTTP errors are user-facing: name the failing field/limit, suggest the next action when obvious ("envelope exceeds CRASHBOX_MAX_ENVELOPE_BYTES=…").

## Security

- Passwords via `argon2` (or equivalent vetted crate). Never store plaintext, never log it.
- Sessions are server-side (table `sessions`) with cookie ids; `CRASHBOX_COOKIE_SECURE` toggles `Secure` for HTTPS deploys.
- Admin APIs require authentication. Ingestion endpoint is public by DSN public-key, **not** by session.
- Public signup is **off** by default (`CRASHBOX_ALLOW_PUBLIC_SIGNUP=false`).
- CORS controlled by `CRASHBOX_CORS_ALLOWED_ORIGINS`. Trust proxy headers only when `CRASHBOX_TRUST_PROXY_HEADERS=true`.
- Rate-limit ingestion per project; reject early with 429 before parsing the body.

## Testing

- `cargo test --workspace` is the gate. `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` must pass. All three are enforced on every push/PR by `.github/workflows/ci.yml`, plus the frontend `pnpm build` (tsc + bundle).
- The Rust toolchain is **pinned** in `backend/rust-toolchain.toml`; CI and local use the same compiler and lint set. Bump it deliberately and fix any new lints in the same change. `unwrap`/`expect` are forbidden in production code (warn-as-error) but allowed in tests via the crate-root / per-test-file `allow`; lint policy lives in `backend/Cargo.toml [lints.clippy]`.
- Unit tests next to the code (`#[cfg(test)] mod tests`) for pure logic — envelope framing, DSN parse, message normalization, fingerprint stability.
- Integration tests in `backend/tests/` for the ingestion path: spin up the app against a temp SQLite file, POST a real captured envelope, assert on stored event + issue.
- **Fixture envelopes** from real Sentry SDKs (browser, Node) live under `backend/tests/fixtures/envelopes/`. New protocol behavior requires a fixture.
- Frontend: keep it simple — type-checked TS, minimal component tests, smoke-test pages via Vitest if it stays cheap.

## Tooling

- Build (dev): `cargo run -p crashbox-backend` + `pnpm dev` (or `npm run dev`) in `frontend/`.
- Build (release): multi-stage Dockerfile — frontend build → embed → `cargo build --release` → minimal runtime image, non-root user, expose `8080`, volume `/data`.
- Lint/format: `cargo clippy` + `cargo fmt`; `eslint` + `prettier` (or `biome`) for the frontend. No alternative formatters.
- DB: SQLx with offline mode (`sqlx prepare`) so the container build doesn't need a live DB. Migrations are checked in under `backend/migrations/` and applied on startup.
- All user-visible config is documented in `docs/configuration.md` — every new `CRASHBOX_*` env var lands there in the same PR.

## Session continuity

Work spans multiple sessions. State lives in two places, in order:

1. **`tsk` MCP** — machine state of every task (`tsk list`, `tsk list --inprogress`, `tsk show <id>`). Use it instead of TODO comments.
2. **`kungfu memory_*`** — project-local decisions, gotchas, conventions ("why grouping diverges from Sentry here", "why we chose argon2 params X"). The canonical store. Search before implementing; add after closing anything non-trivial.

Resume protocol at the start of a new session:

```bash
tsk list --inprogress              # anything mid-flight
tsk list                           # full pending queue
git log --oneline -20              # recent commits
cargo build --workspace            # does it still compile?
# deep-dive on demand:
#   kungfu memory_search "<topic>"
#   kungfu memory_list
```

Maintenance after closing a non-trivial task:

1. `tsk done <id>` (or `tsk update` if scope shifted).
2. New decision, gotcha, or convention → `kungfu memory_add`. Pin sparingly.
3. New `CRASHBOX_*` env var, route, or migration → update `docs/configuration.md` / README in the same PR.
4. User-visible change → bump the version in `backend/Cargo.toml` (and frontend `package.json` if relevant).

## Product guardrails (do not drift)

These are hard MVP constraints from `crashbox_master_prompt.md`. Reject scope that violates them unless the user explicitly opts in.

- **Single container, SQLite default.** No external queue, no ClickHouse/Kafka/Redis. Postgres is *optional, later*.
- **Only the DSN changes** on the SDK side. If a change would require touching user app code beyond the DSN, it's out of scope.
- **No full Sentry clone.** No orgs/teams/RBAC, no performance traces, no session replay, no source maps, no cron monitors, no release health. If asked, point at the master prompt and confirm scope before building.
- **Raw JSON preservation beats premature normalization.** When in doubt, store and move on.
- **Documented limitation > fake compatibility.** If we don't fully implement a Sentry behavior, say so in `docs/protocol.md`.

## Development order (vertical slices)

Follow the slices from `crashbox_master_prompt.md` §16 — bootable backend → bootstrap → ingestion → grouping → UI → production basics. Don't start slice N+1 while slice N has an open acceptance criterion.
