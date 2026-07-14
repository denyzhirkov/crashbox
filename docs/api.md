# HTTP API reference

Everything the Crashbox UI can do, an API client can do. This page is the complete admin API
surface — designed to be equally usable by a human with `curl` and by an automation agent
(CI scripts, Claude Code, cron jobs).

Ingestion endpoints (`/api/:project_id/envelope`, `/api/:project_id/logs`, `/ping/:key`) are
documented separately in [`protocol.md`](protocol.md), [`logs.md`](logs.md), and
[`heartbeats.md`](heartbeats.md) — they authenticate by DSN key / ping key, not by the
credentials below.

## Authentication

Two interchangeable credentials; every endpoint below accepts either unless marked
**session-only**:

- **Session cookie** — `POST /api/auth/login`, then the `crashbox_session` cookie rides along.
  This is what the UI uses.
- **Bearer token** — `Authorization: Bearer cbx_…`. Issue one at `POST /api/tokens`
  (see [`api-tokens.md`](api-tokens.md)). Tokens have a **scope**: `full` (default) or `read`
  (GET/HEAD only — any write returns `403`).

```bash
# login (cookie jar)
curl -c jar -X POST https://crash.example.com/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"admin@example.com","password":"…"}'

# or bearer
curl -H 'Authorization: Bearer cbx_…' https://crash.example.com/api/projects
```

## Conventions

- All bodies are JSON; errors are always `{"error": "<human-readable reason>"}` with a
  meaningful HTTP status (`400` invalid input, `401` no/bad credentials, `403` forbidden or
  read-only scope, `404` missing, `409` conflict, `429` rate-limited).
- **List endpoints return `{"items": [...], "total": <n>}`** — `total` counts every match of
  the current filter, not just the page. Page with `limit` (default 50, max 500) and `offset`.
- Timestamps are ISO-8601 UTC strings.

## Auth

| Method & path        | Notes |
|----------------------|-------|
| `POST /api/auth/login`  | `{email, password}` → sets the session cookie |
| `POST /api/auth/logout` | Clears the session |
| `GET /api/auth/me`      | Current user + instance flags (`live_logs_enabled`, …). Works with bearer — handy self-check for automation |

## API tokens (session-only)

A bearer token can never mint or revoke tokens.

| Method & path            | Notes |
|--------------------------|-------|
| `GET /api/tokens`        | Metadata only — never the secret |
| `POST /api/tokens`       | `{name, scope?: "full"\|"read", expires_in_days?}` → `201` with the plaintext `token`, shown exactly once |
| `DELETE /api/tokens/:id` | Instant revocation |

## Projects

| Method & path                        | Notes |
|--------------------------------------|-------|
| `GET /api/projects`                  | Bare array (small, unpaginated) |
| `GET /api/projects/overview`         | Projects + 24h event counts + recent issues |
| `POST /api/projects`                 | Admin. `{name, platform?}` |
| `GET /api/projects/:id`              | |
| `PATCH /api/projects/:id`            | Admin. `{name?, platform?}` |
| `GET /api/projects/:id/dsn`          | DSN + public key |
| `POST /api/projects/:id/rotate-key`  | Admin. Invalidates the old DSN key |

## Issues

| Method & path                        | Notes |
|--------------------------------------|-------|
| `GET /api/projects/:id/issues`       | Paginated. Filters below |
| `GET /api/issues/:id`                | |
| `PATCH /api/issues/:id`              | `{status?: "resolved"\|"unresolved", snooze?: "1h"\|"1d"\|"1w"\|"forever"\|"wake"}` |
| `PATCH /api/issues`                  | Bulk: `{ids: [1,2,…], status?, snooze?}` → `{"updated": n}`. Max 500 ids |
| `DELETE /api/issues/:id`             | Admin. Deletes the issue **and all its events** |
| `GET /api/issues/:id/events`         | Paginated, newest first |
| `GET /api/events/:id`                | `{event, data}` — `data` is the verbatim SDK payload (`raw_json` parsed) |

**Issue list filters** (combine freely):

| Param | Values |
|-------|--------|
| `status` | `unresolved` (default, hides snoozed) · `resolved` · `snoozed` · `all` |
| `level` | `error`, `warning`, … |
| `environment`, `release` | matched against the issue's events |
| `query` | case-insensitive substring on the issue title |
| `tag` | `tag=key=value`, repeatable (ANDed) |
| `sort` | `last_seen` (default) · `first_seen` · `event_count` |
| `order` | `desc` (default) · `asc` |
| `limit`, `offset` | pagination |

## Events (project-wide feed + full-text search)

```
GET /api/projects/:id/events?q=<full-text>&level=…&environment=…&limit=…&offset=…
```

Paginated, newest first. `q` is **full-text over the raw event payload** (SQLite FTS5): it
reaches stack-frame filenames, function names, breadcrumb messages, request URLs — everything
the SDK sent, not just the indexed columns. Terms are ANDed; FTS operator characters are
treated as literals. Each row carries `issue_id`, so a text hit leads straight to its issue.

```bash
# "which issues mention checkout.js anywhere in the payload?"
curl -H "$AUTH" 'https://crash.example.com/api/projects/1/events?q=checkout.js&limit=5'
```

## Heartbeats

| Method & path                          | Notes |
|-----------------------------------------|-------|
| `GET /api/projects/:id/heartbeats`      | Bare array; each monitor includes its `ping_url` |
| `POST /api/projects/:id/heartbeats`     | Admin. `{name, period_seconds, grace_seconds?, description?}` |
| `PATCH /api/heartbeats/:id`             | Admin. Fields above + `{status: "paused"\|"pending"}` to pause/resume |
| `DELETE /api/heartbeats/:id`            | Admin. Invalidates the ping URL |
| `GET /api/heartbeats/:id/history`       | Paginated status transitions, newest first: `{from_status, to_status, at}`. Depth bounded by `CRASHBOX_RETENTION_DAYS` |

## Live logs (only when `CRASHBOX_LIVE_LOGS_ENABLED=true`)

| Method & path                          | Notes |
|-----------------------------------------|-------|
| `GET /api/projects/:id/logs/stream`     | SSE tail: scrollback replay, then live records |
| `GET /api/projects/:id/logs/recent`     | One-shot snapshot of the in-RAM scrollback → `{"items": […], "count": n}`. Same filters as the stream (`level`, `logger`, `q`) plus `limit` (keeps the newest N). Built for API clients that want current logs without holding a stream open |

Logs are RAM-only — `recent` returns what the ring buffer holds right now, not history.

## Admin

| Method & path | Notes |
|---------------|-------|
| `GET /api/admin/backup` | Streams an atomic snapshot of the SQLite database (`VACUUM INTO`), `application/octet-stream` with a dated `crashbox-YYYYMMDD-HHMMSS.db` filename. Works with read-scope tokens (GET). One backup at a time — a concurrent request gets `409`. The temp snapshot is written next to the live DB and deleted after streaming. |

Backup recipe with an API token:

```bash
curl -fsS -H "Authorization: Bearer cbx_…" -o crashbox-backup.db \
  https://crash.example.com/api/admin/backup
```

(The same snapshot is available offline via the CLI: `docker exec crashbox crashbox backup /data/snap.db`.)

## Health & metrics (unauthenticated)

| Method & path | Notes |
|---------------|-------|
| `GET /healthz` | Liveness |
| `GET /readyz`  | Readiness (DB reachable) |
| `GET /metrics` | Prometheus format |

## Changelog

- **1.9.0** — list endpoints switched from bare arrays to `{items, total}` (breaking);
  added `sort`/`order` on issues, the project-wide `/events` feed with FTS5 search,
  `/logs/recent`, heartbeat `/history`, bulk issue PATCH, issue DELETE, and token scopes.
