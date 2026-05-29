# Crashbox

[![Docker Hub](https://img.shields.io/docker/v/denyzhirkov/crashbox?label=docker%20hub&sort=semver)](https://hub.docker.com/r/denyzhirkov/crashbox)
[![Image size](https://img.shields.io/docker/image-size/denyzhirkov/crashbox/1.0.0?label=image)](https://hub.docker.com/r/denyzhirkov/crashbox/tags)

> A tiny self-hosted Sentry-compatible error tracking server for small projects. It accepts events from existing Sentry SDKs by changing only the DSN, stores them locally in SQLite, groups them into issues, and gives you a simple web UI to inspect crashes.

Crashbox implements a **practical subset** of Sentry ingestion focused on error events. It is **not** a full Sentry replacement and does not implement performance traces, session replay, source maps, attachments, organizations, or any of the heavier observability features. See [`docs/protocol.md`](docs/protocol.md) for the explicit list of what's in and out.

The project feels like *"I need error tracking for my small app tonight"*, not *"I need to operate an observability platform"*.

---

## Quick start (Docker)

```bash
docker run -d \
  --name crashbox \
  -p 8080:8080 \
  -v crashbox-data:/data \
  -e CRASHBOX_ADMIN_EMAIL=admin@example.com \
  -e CRASHBOX_ADMIN_PASSWORD=change-me-on-first-boot \
  -e CRASHBOX_PROJECT_NAME=my-app \
  -e CRASHBOX_PUBLIC_URL=http://localhost:8080 \
  denyzhirkov/crashbox:1.2.0
```

Image: [`denyzhirkov/crashbox`](https://hub.docker.com/r/denyzhirkov/crashbox) on Docker Hub (~40 MB, distroless, non-root, multi-arch `linux/amd64` + `linux/arm64`). Pin to a specific tag (`:1.2.0`) in production; `:latest` follows the most recent release.

Watch the logs for the bootstrap line — it prints the DSN exactly once:

```
INFO bootstrap: project DSN (shown once at startup) dsn=http://<key>@localhost:8080/1
```

Point an unmodified Sentry SDK at it:

```ts
import * as Sentry from "@sentry/node"
Sentry.init({ dsn: "http://<key>@localhost:8080/1" })
Sentry.captureException(new Error("hello crashbox"))
```

Open `http://localhost:8080`, log in with the email/password from above, and the event will be there. That's the whole thing.

---

## Why

You have a side project, an internal tool, or a small SaaS. You want stacktraces of production errors without:

- running the full Sentry self-hosted stack (Postgres + Redis + Kafka + ClickHouse + Snuba + Symbolicator + a handful of Python services),
- paying $26/mo per user,
- or writing your own SDK that nobody else integrates with.

Crashbox is one ~40 MB Docker image, one SQLite file, no background services beyond the cleanup tick. It runs comfortably on the smallest VPS you can rent.

---

## What's included

- **Sentry envelope ingestion** at `POST /api/:project_id/envelope[/]` — works with the official `@sentry/browser`, `@sentry/node`, and any SDK that uses the standard DSN/envelope format.
- **Issue grouping** — same exception groups together even when error messages contain variable IDs / UUIDs / long hashes. Custom `fingerprint` in the event payload is honored.
- **Web UI** in SolidJS — Login, Projects, Issues list (with filters), Issue detail (stack trace + breadcrumbs + tags + user + request + raw JSON), Settings. Warm-dark default. Keyboard nav (`j`/`k`/`o`/`/`).
- **Auth** — server-side sessions, argon2 password hashing, single admin user from env. No public signup.
- **Retention job** — deletes old events while keeping the last N per issue. Issue summaries live forever.
- **One Rust binary**, one SQLite file, embedded frontend. The whole production image is **~40 MB**.

See `docs/` for the details:

- [`docs/protocol.md`](docs/protocol.md) — the exact Sentry ingest subset we accept
- [`docs/configuration.md`](docs/configuration.md) — every `CRASHBOX_*` env var
- [`docs/development.md`](docs/development.md) — local dev, tests, layout
- [`docs/ui-design.md`](docs/ui-design.md) — UI brief / design notes

---

## Building from source

```bash
# Backend
cd backend && cargo build --release && cd ..

# Frontend (must be built BEFORE the backend if you want it embedded)
cd frontend && pnpm install && pnpm build && cd ..

# Or just do the whole thing in Docker
docker build -t crashbox:local .
```

---

## Verifying compatibility with your SDK

A working `@sentry/node` example is at [`examples/sentry-node/`](examples/sentry-node/). Run it against a live Crashbox:

```bash
cd examples/sentry-node
pnpm install
DSN=http://<key>@localhost:8080/1 node send.js
```

A working `@sentry/browser` example is at [`examples/sentry-browser/index.html`](examples/sentry-browser/index.html). Open it in a browser (or via `python3 -m http.server`) and click the button.

In both cases, the event should appear in the UI within a second.

---

## Status

**1.2.0** — multi-arch release: the Docker image now ships as a manifest list covering `linux/amd64` and `linux/arm64` (1.1.0 was arm64-only and failed with `exec format error` on amd64 hosts). Releases are now built and published by GitHub Actions on tag push. No application changes.

**1.1.0** — feature release: webhooks (Telegram/Discord/generic) with spike detection, auto-resolve & snooze, 24h sparklines per issue, Cmd+K command palette, tag click-to-filter + saved views, admin CLI (`crashbox projects list`, etc.), Prometheus `/metrics`. Single-tenant, single-user, single binary. Published to Docker Hub as [`denyzhirkov/crashbox`](https://hub.docker.com/r/denyzhirkov/crashbox). Use at your own risk on something small first.

## Acceptance

Spec-derived acceptance, verified live in a Docker container:

| # | Criterion | Verified |
|---|---|---|
| 1 | Runs via Docker with one command | ✅ |
| 2 | Creates admin account from env vars | ✅ |
| 3 | Creates default project from env vars | ✅ |
| 4 | Displays/copies the project DSN | ✅ (logged once, also in `/api/projects/:id/dsn` and Settings UI) |
| 5 | Sentry JavaScript browser SDK works by changing only DSN | ✅ (`examples/sentry-browser`) |
| 6 | Sentry Node.js SDK works by changing only DSN | ✅ (verified with `@sentry/node@8.55.2`) |
| 7 | Events appear in UI | ✅ |
| 8 | Similar events grouped into one issue | ✅ (3 sends → 1 issue, `event_count=3`) |
| 9 | Issues can be resolved / unresolved | ✅ (`PATCH /api/issues/:id`) |
| 10 | Old events cleaned by retention | ✅ (4 retention tests cover age + floor + orphans + within-window) |
| 11 | Service survives malformed envelopes | ✅ (5 ingest tests cover wrong key, garbage body, truncated payloads, etc.) |
| 12 | README explains setup clearly | ✅ (this file) |

## License

MIT or Apache-2.0 at your option.
