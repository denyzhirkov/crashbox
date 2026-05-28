# Development

## Prerequisites

- Rust ≥ 1.80 (we use `2021` edition with stable APIs only)
- Node.js ≥ 22 + `pnpm` (via `corepack enable`)
- Docker (optional, for the production-shape build)

## Local dev — two-process

Best for fast iteration on the UI:

```bash
# terminal 1 — backend
cd backend
CRASHBOX_PORT=8080 \
CRASHBOX_DATABASE_URL=sqlite://../data/crashbox.db \
CRASHBOX_ADMIN_EMAIL=admin@example.com \
CRASHBOX_ADMIN_PASSWORD=hunter2 \
CRASHBOX_PROJECT_NAME=dev \
CRASHBOX_PROJECT_PUBLIC_KEY=devkey \
cargo run

# terminal 2 — frontend
cd frontend
pnpm install
pnpm dev
# open http://localhost:5173 — Vite proxies /api to :8080
```

The frontend's `vite.config.ts` proxies all `/api/*` requests to `http://localhost:8080`, so
auth cookies and live data work as if served from the same origin.

## Local dev — single-process (production shape)

To exercise the embedded-SPA path:

```bash
# build the frontend first
cd frontend && pnpm install && pnpm build && cd ..

# then run the backend — rust-embed picks up frontend/dist
cd backend && cargo run
# open http://localhost:8080
```

## Tests

```bash
cd backend
cargo test --workspace          # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Test counts: 58 (46 unit + 5 ingest + 3 admin_api + 4 retention).

Tests use real SQLite in temp dirs — no in-memory database, no mocks. Integration tests spin up
a full Axum server on `127.0.0.1:0`, login via reqwest's cookie store, and assert against
actual HTTP responses.

### Adding fixtures

If you want to validate a new SDK or version against Crashbox:

1. Capture the raw envelope/event JSON from the SDK (see `examples/sentry-node`).
2. Drop it under `backend/tests/fixtures/envelopes/<sdk>-<scenario>.event.json`.
3. Add an integration test that posts it through `/api/:project_id/envelope/` and asserts on
   the resulting issue/event row.

## Docker

```bash
docker build -t crashbox:local .
docker run --rm -p 8080:8080 \
  -e CRASHBOX_ADMIN_EMAIL=admin@example.com \
  -e CRASHBOX_ADMIN_PASSWORD=change-me \
  -e CRASHBOX_PROJECT_NAME=demo \
  crashbox:local
```

Resulting image is ~40 MB (distroless/cc + statically-linked SQLite + LTO + strip).

Or via compose:

```bash
docker compose up --build
```

## Project layout

```
backend/
  Cargo.toml            single crate, no workspace
  migrations/           SQLx migrations applied on startup
  src/
    main.rs             entrypoint, tracing init, graceful shutdown
    lib.rs              public module map (for integration tests)
    config.rs           CRASHBOX_* env → validated Config
    app_state.rs        Arc<Config> + db pool + rate limiter
    bootstrap.rs        idempotent admin/project creation
    db/                 SQLx repositories
    http/               Axum handlers + routes + assets (rust-embed)
    ingest/             rate limiter
    jobs/cleanup.rs     retention sweep
    security/           argon2 + sessions
    sentry/             envelope parser, auth, normalize, grouping, dsn
  tests/                integration tests (ingest, admin_api, retention)
  tests/fixtures/       real SDK envelopes
frontend/
  src/api/              typed fetch wrapper + shared types
  src/components/       AppShell, EdgeBar, RequireAuth
  src/lib/              auth context, theme, time formatting
  src/pages/            Login, Projects, Issues, IssueDetail, Settings
docs/                   protocol.md, configuration.md, development.md, ui-design.md
examples/               sentry-node, sentry-browser
Dockerfile docker-compose.yml .dockerignore
```

## Adding HTTP routes

1. Add a handler in `backend/src/http/<module>.rs`.
2. Register it in `backend/src/http/routes.rs::build`.
3. Return `AppResult<T>` (or `impl IntoResponse`); use `AppError` variants — they map to HTTP
   status codes automatically.
4. Add an integration test under `backend/tests/`.

## Adding a CRASHBOX_* setting

1. Add the field to the appropriate sub-struct in `backend/src/config.rs` (`Config`,
   `IngestLimits`, etc.).
2. Read it in `Config::from_env` using `env_or` / `parse_env` / `env_opt`.
3. Document it in `docs/configuration.md` in the same PR.
4. Use it from wherever; `AppState.config` is an `Arc<Config>` so cloning is cheap.

## Adding a frontend page

1. New file under `frontend/src/pages/`.
2. Add a route in `frontend/src/App.tsx`.
3. Use `createResource` for data; `useAuth()` for the current user; the API client at
   `src/api/client.ts` exports `api.*` for all endpoints with typed returns.
4. Mind the design notes in [`ui-design.md`](./ui-design.md) — keep mono-first, no chip-soup,
   keep the edge-bar pattern for severity.
