# Configuration

Every Crashbox setting is an environment variable with the `CRASHBOX_` prefix. All variables are
optional unless marked **required**; defaults are shown.

The Rust binary reads them on startup, validates them, and fails loudly if anything is malformed.

## Core server

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_HOST` | `0.0.0.0` | Bind address. Use `127.0.0.1` if you put a reverse proxy in front. |
| `CRASHBOX_PORT` | `8080` | TCP port to listen on. |
| `CRASHBOX_PUBLIC_URL` | `http://localhost:8080` | Base URL used to compose project DSNs. Set this to whatever public address SDKs will use. |
| `CRASHBOX_DATABASE_URL` | `sqlite://./data/crashbox.db` | SQLite path. In the docker image the default is `sqlite:///data/crashbox.db`. Postgres is not supported in MVP. |
| `CRASHBOX_DATA_DIR` | `./data` | Reserved for future use (attachments, etc.). |
| `CRASHBOX_LOG_LEVEL` | `info` | Falls back to `info` if unparseable. Use `CRASHBOX_LOG_FILTER` for full tracing-subscriber syntax (`crashbox=debug,sqlx=warn`). |
| `CRASHBOX_SECRET_KEY` | `change-me-generate-random` | Reserved for future cookie/CSRF signing. Set to a long random string in any deployment. |

## Admin bootstrap

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_ADMIN_EMAIL` | _(unset)_ | First-boot admin email. **Required** for first-run admin creation. |
| `CRASHBOX_ADMIN_PASSWORD` | _(unset)_ | First-boot admin password. Hashed with argon2 on insert. |
| `CRASHBOX_ADMIN_NAME` | _(unset)_ | Optional display name. |
| `CRASHBOX_FORCE_ADMIN_RESET` | `false` | When `true`, resets the admin password on next boot to the value of `CRASHBOX_ADMIN_PASSWORD`. Remove the env after using it. |

Bootstrap is idempotent: if the admin already exists and `CRASHBOX_FORCE_ADMIN_RESET=false`,
nothing changes.

## Project bootstrap

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_PROJECT_NAME` | _(unset)_ | Name of the default project, created on first boot when no projects exist. |
| `CRASHBOX_PROJECT_PLATFORM` | _(unset)_ | Hint shown in the UI (e.g. `javascript`, `python`, `node`). |
| `CRASHBOX_PROJECT_ENVIRONMENT` | _(unset)_ | Default environment for issues display. |
| `CRASHBOX_PROJECT_PUBLIC_KEY` | _(unset)_ | If set, used as the DSN's public key. If unset, a ULID is generated. |
| `CRASHBOX_PROJECT_SECRET_KEY` | _(unset)_ | If set, hashed and stored. Not used for ingestion in MVP. |

The DSN is logged once at startup using `INFO` level. Subsequent logs mask the public key.

## Ingestion limits

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_MAX_ENVELOPE_BYTES` | `1048576` (1 MiB) | Body size cap; enforced via tower-http `DefaultBodyLimit` before parsing. |
| `CRASHBOX_MAX_EVENT_BYTES` | `524288` (512 KiB) | Per-item event payload cap. |
| `CRASHBOX_MAX_EVENTS_PER_MINUTE_PER_PROJECT` | `600` | In-memory token bucket per project (10/sec). Exceeded → `429` with `Retry-After: 1`. |
| `CRASHBOX_ACCEPT_UNKNOWN_ITEM_TYPES` | `false` | Reserved for future raw-item store. |
| `CRASHBOX_STORE_RAW_UNSUPPORTED_ITEMS` | `false` | Reserved. |
| `CRASHBOX_ENABLE_LEGACY_STORE_ENDPOINT` | `false` | Reserved; `/api/:project_id/store/` not in MVP. |

## Retention

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_RETENTION_DAYS` | `30` | Events older than this are eligible for deletion. |
| `CRASHBOX_MAX_EVENTS_PER_ISSUE` | `100` | **Floor**, not cap: the N most-recent events per issue are protected from age-based deletion. |
| `CRASHBOX_CLEANUP_INTERVAL_SECONDS` | `3600` | How often the retention sweep runs. Set to `0` to disable the job entirely. |

Issue summary rows are **never** deleted by retention — only event rows expire.

## UI

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_UI_ENABLED` | `true` | Reserved; UI is always served in MVP. |
| `CRASHBOX_UI_APP_NAME` | `Crashbox` | Reserved for future white-labeling. |
| `CRASHBOX_UI_THEME` | `system` | **Deprecated.** UI is dark-only; this var is ignored. |

## Security

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_COOKIE_SECURE` | `false` | Set to `true` behind HTTPS — adds the `Secure` flag to the session cookie. |
| `CRASHBOX_CORS_ALLOWED_ORIGINS` | `*` | Reserved; CORS layer not enforced in MVP (single-origin UI). |
| `CRASHBOX_TRUST_PROXY_HEADERS` | `false` | Reserved for `X-Forwarded-*` parsing. |
| `CRASHBOX_ALLOW_PUBLIC_SIGNUP` | `false` | Hard off in MVP; no signup route exists. |

## Notifications (post-MVP)

| Variable | Default |
|---|---|
| `CRASHBOX_TELEGRAM_BOT_TOKEN` | _(unset)_ |
| `CRASHBOX_TELEGRAM_CHAT_ID` | _(unset)_ |
| `CRASHBOX_DISCORD_WEBHOOK_URL` | _(unset)_ |
| `CRASHBOX_GENERIC_WEBHOOK_URL` | _(unset)_ |

Currently accepted by `Config::from_env` but not wired anywhere. Setting them has no effect yet.

## Advanced logging

`CRASHBOX_LOG_FILTER` overrides `CRASHBOX_LOG_LEVEL` and accepts the full
[`tracing-subscriber` EnvFilter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)
syntax:

```bash
CRASHBOX_LOG_FILTER="crashbox=debug,sqlx::query=warn"
```

Useful when investigating ingest issues without flooding logs from SQLx query traces.

## What is NOT configurable

- The list of HTTP routes.
- The grouping algorithm (see `docs/protocol.md`).
- Argon2 parameters (uses crate defaults).
- The 30-day session TTL.
- The 100-tag / 200-breadcrumb per-event caps.

If you need to change any of these, edit the code and rebuild.
