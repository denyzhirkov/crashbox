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
  denyzhirkov/crashbox:1.6.0
```

Image: [`denyzhirkov/crashbox`](https://hub.docker.com/r/denyzhirkov/crashbox) on Docker Hub (~40 MB, distroless, non-root, multi-arch `linux/amd64` + `linux/arm64`). Pin to a specific tag (`:1.6.0`) in production; `:latest` follows the most recent release.

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
- **Live Logs** — a separate, ephemeral real-time log channel alongside errors. Stream ordinary application logs to a project and tail them live in the UI with filtering, grouping and a throughput sparkline. RAM-only, never written to disk. See below.
- **Heartbeats (dead-man's switch)** — register a cron job or service that must ping you every N seconds; if the ping doesn't arrive in time, Crashbox flips the monitor to `down` and alerts. One `curl` at the end of the cron line is the whole integration. See below.
- **Web UI** in SolidJS — Login, Projects, Issues list (with filters), Issue detail (stack trace + breadcrumbs + tags + user + request + raw JSON), Settings. Warm-dark default. Keyboard nav (`j`/`k`/`o`/`/`).
- **Auth** — server-side sessions, argon2 password hashing, single admin user from env. No public signup.
- **API tokens** — mint a bearer token in the UI, hand it to a script / CI / Claude Code for full API automation (`Authorization: Bearer cbx_…`), revoke it in one click. Hash-at-rest, shown once, tokens can't manage tokens.
- **Retention job** — deletes old events while keeping the last N per issue. Issue summaries live forever.
- **One Rust binary**, one SQLite file, embedded frontend. The whole production image is **~40 MB**.

See `docs/` for the details:

- [`docs/protocol.md`](docs/protocol.md) — the exact Sentry ingest subset we accept
- [`docs/configuration.md`](docs/configuration.md) — every `CRASHBOX_*` env var
- [`docs/logs.md`](docs/logs.md) — Live Logs protocol, streaming, and limitations
- [`docs/heartbeats.md`](docs/heartbeats.md) — heartbeat monitors: ping contract, states, alerts
- [`docs/api-tokens.md`](docs/api-tokens.md) — bearer tokens for automation: issue, use, revoke
- [`docs/development.md`](docs/development.md) — local dev, tests, layout
- [`docs/ui-design.md`](docs/ui-design.md) — UI brief / design notes

---

## Live Logs

Crashbox tracks two separate things, and keeps them separate on purpose:

| | **Events** | **Live Logs** |
|---|---|---|
| For | errors & crashes | ordinary "what's happening now" logs |
| Parsed / grouped into issues | yes | no |
| Stored | **persisted in SQLite** | **RAM only — never touches disk** |
| Retention | retention job + history | last *N* lines per project, gone on restart |
| Consumed via | the dashboard issue views | a live tail (SSE) |

Live Logs is a real-time tail: open a project's log view, watch lines stream in, filter / group them, close it. It adds **no new infrastructure** — no queue, no extra datastore. Just an in-memory ring buffer plus a broadcast channel inside the same single binary. It is intentionally ephemeral and lossy: under load (or for a slow viewer) lines are dropped rather than queued, and a restart clears everything.

### Sending logs

Authenticated by your **DSN public key** — the same credential events already use, so the only thing that changes on your side is where logs are pointed. Two formats are accepted:

1. **Dedicated endpoint** `POST /api/:project_id/logs` — a JSON array, a single object, or NDJSON:

   ```bash
   curl -X POST "http://localhost:8080/api/1/logs" \
     -H "X-Sentry-Auth: Sentry sentry_key=YOUR_PUBLIC_KEY" \
     --data-binary $'{"level":"info","message":"boot ok"}\n{"level":"warn","message":"slow query","logger":"db","attrs":{"ms":820}}\n'
   ```

   Recognized fields: `level` (`trace`→`fatal`, default `info`), `message` (or `msg`/`body`), `logger`, `ts` (ISO-8601 or epoch seconds), `trace_id`; everything else is collected into `attrs`. Bad lines are skipped, not fatal. Responds `202 {"accepted":N,"skipped":M}`.

2. **Sentry `log` envelope item** — if your SDK already emits Sentry structured logs, they ride in on the normal `/envelope/` request and Crashbox routes the `log` items into Live Logs automatically.

### Watching logs

In the dashboard, open a project and click **live logs** (or `⌘K → view live logs`). The page connects over Server-Sent Events, replays recent scrollback, then streams live. You get a severity floor, free-text search across message/logger/attrs, **pause/resume** (with a buffered "N new lines" pill), **group-by-logger**, a 60-second throughput **sparkline**, and auto-scroll.

### Turning it off

Set `CRASHBOX_LIVE_LOGS_ENABLED=false` and the ingest + stream routes are not mounted and the UI hides the section entirely. All limits are bounded and configurable (`CRASHBOX_LIVE_LOG_*`, `CRASHBOX_MAX_LOG_*`) — see [`docs/configuration.md`](docs/configuration.md). Protocol details and the full list of limitations are in [`docs/logs.md`](docs/logs.md).

> **Heads-up:** because logs live only in the receiving instance's RAM, Live Logs assumes the single-container deployment Crashbox is built for. Behind multiple replicas a viewer only sees the replica its stream landed on.

---

## Heartbeats (dead-man's switch)

Error tracking only sees failures the process survives long enough to report. Heartbeats catch the other kind: the cron job that never started, the backup script on a dead box, the worker that silently stopped. You register what *should* run and how often; the job pings a per-monitor URL after each run; **silence past the deadline becomes the alert.**

```sh
# the entire integration — end of any cron line / script:
pg_dump mydb | gzip > backup.gz && curl -fsS https://crash.example.com/ping/<ping_key>
```

- Monitors live next to a project's issues in the UI: status badge (`pending / up / down / paused`), last ping, a live "due in / overdue by" countdown, pause/resume, and the ping URL with one-click copy.
- `GET` or `POST`, no auth beyond the unguessable key, no body — anything that can make an HTTP request can be monitored.
- Alerts (`heartbeat_down` with how overdue, `heartbeat_recovered` with the downtime) go through the same notification channels as issue alerts — Telegram, Discord, generic webhook. One alert per transition, no repeat nagging.
- Purely **passive**: Crashbox never probes your services. Active HTTP uptime checks need an external vantage point and are out of scope on purpose.

The sweep runs inside the same single binary (default every 30 s, `CRASHBOX_HEARTBEAT_SWEEP_INTERVAL_SECONDS`); pings are rate-limited per monitor (`CRASHBOX_HEARTBEAT_MAX_PINGS_PER_MINUTE`). Contract, state machine, and limitations: [`docs/heartbeats.md`](docs/heartbeats.md).

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

**1.6.0** — feature release: **API tokens** — personal bearer tokens for automation: mint in the UI (shown once, SHA-256 at rest, optional expiry), use on every admin endpoint via `Authorization: Bearer cbx_…`, revoke instantly; token endpoints are session-only so a leaked token can't mint successors. Built for handing Crashbox to scripts and AI agents (Claude Code) — see [`docs/api-tokens.md`](docs/api-tokens.md). Also fixes icon rendering on rows after the first (Solid shared-DOM-node bug on the heartbeats tape) and unifies project navigation: every project page now shows the same section tabs (`issues · live logs · heartbeats · settings`) with the current one highlighted, instead of each page hand-rolling an inconsistent link list.

**1.5.0** — feature release: **Heartbeats (dead-man's switch)** — register jobs that must ping `GET|POST /ping/<key>` every `period_seconds`; a sweep flips silent monitors to `down` (once per transition) and sends `heartbeat_down` / `heartbeat_recovered` through the existing notification channels with overdue/downtime detail. New dashboard page with status badges, live due/overdue countdown, create/edit, pause/resume, and one-click ping-URL copy. Per-monitor ping rate limit, `CRASHBOX_HEARTBEAT_*` env vars, Prometheus counters `crashbox_heartbeat_pings_total` / `crashbox_heartbeat_transitions_total`. Passive by design — no outbound probing. See [`docs/heartbeats.md`](docs/heartbeats.md).

**1.4.0** — feature release: **Live Logs** — an ephemeral, RAM-only real-time log channel alongside error tracking. Dedicated `POST /api/:id/logs` ingest (JSON array / object / NDJSON) plus recognition of Sentry `log` envelope items, both DSN-authed; per-project in-memory ring buffer with a lossy broadcast; session-authed SSE stream at `GET /api/projects/:id/logs/stream` with server-side level/logger/text filters; a new dashboard page with severity floor, search, pause/resume, group-by-logger, and a throughput sparkline. Gated by `CRASHBOX_LIVE_LOGS_ENABLED`; new Prometheus metrics `crashbox_livelog_received_total` / `_dropped_total` / `_active_subscribers`. Nothing is persisted to disk. See [`docs/logs.md`](docs/logs.md).

**1.3.0** — UI redesign to the Signal/Aurora design system (dark-only, violet→cyan accent, frosted-glass cards). No backend changes.

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
