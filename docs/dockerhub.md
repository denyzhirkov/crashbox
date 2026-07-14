# crashbox

> A tiny self-hosted Sentry-compatible error tracking server.
> One Rust binary, one SQLite file, an embedded SolidJS web UI, ~40 MB distroless image.

Crashbox accepts events from **unmodified** official Sentry SDKs (`@sentry/node`, `@sentry/browser`, …) by changing only the DSN. No custom SDK, no Postgres+Redis+Kafka+ClickHouse stack — just one container.

**Not** a full Sentry replacement: no performance traces, session replay, source maps, attachments, orgs/teams. If you need any of those, look elsewhere. If you want an error inbox for your side project tonight — this is it.

[**Source on GitHub**](https://github.com/denyzhirkov/crashbox) · [Docs](https://github.com/denyzhirkov/crashbox/tree/main/docs)

---

## Quick start

```bash
docker run -d --name crashbox \
  -p 8080:8080 \
  -v crashbox-data:/data \
  -e CRASHBOX_ADMIN_EMAIL=admin@example.com \
  -e CRASHBOX_ADMIN_PASSWORD=change-me \
  -e CRASHBOX_PROJECT_NAME=my-app \
  -e CRASHBOX_PUBLIC_URL=http://localhost:8080 \
  denyzhirkov/crashbox:1.7.0
```

Watch the logs once for the DSN:
```
INFO bootstrap: project DSN (shown once at startup)
     dsn=http://<key>@localhost:8080/1
```

Point any Sentry SDK at it — only the DSN changes:
```ts
import * as Sentry from "@sentry/node"
Sentry.init({ dsn: "http://<key>@localhost:8080/1" })
```

Open `http://localhost:8080`, log in, your crashes are there.

---

## Tags

| Tag | What |
|---|---|
| `denyzhirkov/crashbox:1.7.0` | Pinned 1.7.0 — **recommended** in production |
| `denyzhirkov/crashbox:latest` | Floats to the newest release |

---

## Required environment

| Variable | Notes |
|---|---|
| `CRASHBOX_ADMIN_EMAIL` | Bootstrapped on first boot. Required for first-run admin creation. |
| `CRASHBOX_ADMIN_PASSWORD` | Hashed with argon2 on insert. Set strong; rotate via the CLI later if needed. |
| `CRASHBOX_PROJECT_NAME` | Default project created on first boot. |
| `CRASHBOX_PUBLIC_URL` | Base URL used to compose project DSNs. Set it to whatever address your SDKs will reach. |

## Useful optional environment

| Variable | Default | What |
|---|---|---|
| `CRASHBOX_PORT` | `8080` | Listen port |
| `CRASHBOX_RETENTION_DAYS` | `30` | Events older than this are deleted (last N per issue protected) |
| `CRASHBOX_AUTO_RESOLVE_DAYS` | `14` | Auto-resolve issues with no events for N days (auto-reopens on next event) |
| `CRASHBOX_MAX_EVENTS_PER_MINUTE_PER_PROJECT` | `600` | Per-project ingestion cap |
| `CRASHBOX_GENERIC_WEBHOOK_URL` | _(unset)_ | POST notifications to a URL when new issue / re-open / spike. Telegram + Discord variants also exist. |
| `CRASHBOX_COOKIE_SECURE` | `false` | Set `true` behind HTTPS |

Full reference: [`docs/configuration.md`](https://github.com/denyzhirkov/crashbox/blob/main/docs/configuration.md).

---

## What you get

- **Ingest**: `POST /api/:project_id/envelope[/]` compatible with `@sentry/node`, `@sentry/browser`, and any SDK using the standard DSN/envelope format
- **Web UI**: Login → Projects → Issues list (with filters, search, tag click-to-filter, saved views) → Issue detail (stack trace + breadcrumbs + tags + user + request + raw JSON). Warm-dark theme. **Cmd+K** command palette anywhere. 24h sparkline per issue.
- **Issue management**: resolve / reopen / snooze (1h / 1d / 1w / until-next-crash) / auto-resolve / auto-reopen
- **Alerts**: webhooks for new issue / re-open / **spike detection** (rate jumps 5× over 24h baseline) — Telegram, Discord, or any URL
- **Operations**: built-in CLI (`docker exec crashbox crashbox projects list`, `issues resolve <id>`, `backup /data/snap.db`), Prometheus `/metrics`, atomic SQLite backups via `VACUUM INTO` — also over HTTP: `curl -H "Authorization: Bearer cbx_…" -o snap.db https://crash.example.com/api/admin/backup`
- **Single container**: 40 MB distroless image, non-root, statically-linked SQLite, no shell

---

## Volume

`/data` — SQLite database, WAL file. Mount a named volume or a host path so you don't lose data on container restart.

## Health

`GET /healthz` — liveness · `GET /readyz` — readiness · `GET /metrics` — Prometheus

```yaml
# minimal compose
services:
  crashbox:
    image: denyzhirkov/crashbox:1.7.0
    restart: unless-stopped
    ports: ["8080:8080"]
    volumes: [crashbox-data:/data]
    environment:
      CRASHBOX_PUBLIC_URL: "http://localhost:8080"
      CRASHBOX_ADMIN_EMAIL: "admin@example.com"
      CRASHBOX_ADMIN_PASSWORD: "change-me"
      CRASHBOX_PROJECT_NAME: "my-app"

volumes:
  crashbox-data:
```

---

## Image details

- **Base**: `gcr.io/distroless/cc-debian12:nonroot` — glibc + ca-certificates + a non-root user, no shell
- **Size**: ~40 MB
- **User**: `nonroot:nonroot` (uid 65532)
- **Entry**: `/usr/local/bin/crashbox` (same binary, `serve` is default; runs as CLI for other subcommands)
- **Architectures**: linux/amd64, linux/arm64 (multi-arch manifest list)

## License

MIT or Apache-2.0 at your option.
