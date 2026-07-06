# Configuration

Every Crashbox setting is an environment variable with the `CRASHBOX_` prefix. All variables are
optional unless marked **required**; defaults are shown.

The Rust binary reads them on startup, validates them, and fails loudly if anything is malformed.

## Core server

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_HOST` | `0.0.0.0` | Bind address. Use `127.0.0.1` if you put a reverse proxy in front. |
| `CRASHBOX_PORT` | `8080` | TCP port to listen on. |
| `CRASHBOX_PUBLIC_URL` | `http://localhost:8080` | Base URL used to compose project DSNs, heartbeat ping URLs, and the links inside notifications. Set this to whatever public address SDKs and cron jobs will reach. |
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

## Live Logs

Ephemeral, RAM-only real-time log streaming, separate from durable events. Nothing is persisted — see `docs/logs.md` for the protocol and limitations.

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_LIVE_LOGS_ENABLED` | `true` | Master switch. When `false`, the ingest + stream routes are not mounted and the UI hides the section. |
| `CRASHBOX_LIVE_LOG_BUFFER_PER_PROJECT` | `1000` | Per-project ring buffer size (scrollback replayed to a freshly-connected stream). Held in RAM, lost on restart. |
| `CRASHBOX_MAX_LOG_BATCH_BYTES` | `262144` (256 KiB) | Body cap for `POST /api/:project_id/logs`; enforced via `DefaultBodyLimit` before parsing. |
| `CRASHBOX_LIVE_LOG_MESSAGE_MAX_BYTES` | `16384` (16 KiB) | Per-record message cap; longer messages are truncated on a UTF-8 boundary. |
| `CRASHBOX_MAX_LOGS_PER_MINUTE_PER_PROJECT` | `6000` | Per-project token bucket for log ingest (100/sec). Exceeded → `429` with `Retry-After`. |
| `CRASHBOX_MAX_LOG_SUBSCRIBERS_PER_PROJECT` | `50` | Cap on concurrent SSE subscribers per project; over the cap → `429`. Guards against leaked streams. |

## Retention

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_RETENTION_DAYS` | `30` | Events older than this are eligible for deletion. |
| `CRASHBOX_MAX_EVENTS_PER_ISSUE` | `100` | **Floor**, not cap: the N most-recent events per issue are protected from age-based deletion. |
| `CRASHBOX_CLEANUP_INTERVAL_SECONDS` | `3600` | How often the retention sweep runs. Set to `0` to disable the job entirely. |
| `CRASHBOX_AUTO_RESOLVE_DAYS` | `14` | Auto-flip `unresolved` issues to `resolved` after this many days without a new event. `0` disables. Auto-reopen happens implicitly: if a new event arrives on an auto-resolved fingerprint, the next ingest flips status back to `unresolved` and the notify hub fires a `reopened` alert. |

Issue summary rows are **never** deleted by retention — only event rows expire.

## Spike detection

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_SPIKE_CHECK_INTERVAL_SECONDS` | `300` | How often to scan for spiking issues. `0` disables the job entirely. |
| `CRASHBOX_SPIKE_MIN_EVENTS_PER_HOUR` | `10` | An issue needs at least this many events in the last hour to be considered. Prevents noisy alerts on tiny numbers. |
| `CRASHBOX_SPIKE_RATIO_THRESHOLD` | `5.0` | Current-hour rate must be at least this many times higher than the prior-23h baseline. |
| `CRASHBOX_SPIKE_COOLDOWN_SECONDS` | `3600` | After a spike alert for an issue, suppress further spike alerts for this issue for N seconds. Stored as `issues.spike_alerted_at`. |

Spike alerts go through the same notify channels as `new_issue` / `reopened` (see Notifications below), with `kind=spike` plus `current_hour` and `baseline_per_hour` fields in the JSON payload. Spikes are only computed for issues that already exist (have an `issue_id`) — brand-new bursts are covered by `new_issue` triggers from the ingest path.

The job is also disabled automatically when no notifiers are configured — no point scanning if there's nowhere to send the result.

## Heartbeats

Dead-man's switch monitors: a cron job or service is expected to hit its ping URL (`GET`/`POST /ping/<ping_key>`) every `period_seconds`; once `period + grace` passes with no ping, the monitor flips to `down` and an alert goes out through the notify channels. The next ping flips it back to `up` with a recovery alert.

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_HEARTBEAT_SWEEP_INTERVAL_SECONDS` | `30` | How often the sweep looks for overdue monitors. `0` disables the sweep job (pings are still recorded, but nothing flips to `down`). |
| `CRASHBOX_HEARTBEAT_MAX_PINGS_PER_MINUTE` | `120` | Per-monitor cap on accepted pings; excess pings get `429` with `Retry-After`. |

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

## Notifications

| Variable | Default | Notes |
|---|---|---|
| `CRASHBOX_TELEGRAM_BOT_TOKEN` | _(unset)_ | Set together with `CRASHBOX_TELEGRAM_CHAT_ID`. Both required to enable Telegram delivery. |
| `CRASHBOX_TELEGRAM_CHAT_ID` | _(unset)_ | Target chat or channel ID (negative integer for groups). |
| `CRASHBOX_DISCORD_WEBHOOK_URL` | _(unset)_ | Full webhook URL from Discord's channel settings. |
| `CRASHBOX_GENERIC_WEBHOOK_URL` | _(unset)_ | Any URL — receives a POST with the full `Notification` payload as JSON. Useful for piping into Slack via webhook, PagerDuty, or your own relay. |
| `CRASHBOX_NOTIFY_MAX_PER_MINUTE` | `30` | Per-notifier token-bucket cap. Excess notifications are **dropped** (logged at INFO), not queued. Keeps a sudden burst of new issues from spamming the channel. |

**Triggers.** Notifications fire on issue-level transitions only:

- **`new_issue`** — first event of a previously unseen fingerprint
- **`reopened`** — event arrives on an issue whose status was `resolved`; the status auto-flips back to `unresolved`
- **`spike`** — known unresolved issue is suddenly burning N× hotter than its baseline (see Spike detection above)

Heartbeat monitors ride the same channels with their own payload shape (`monitor_name` instead of issue fields):

- **`heartbeat_down`** — a monitor missed its ping deadline (`last_ping + period + grace`); fires once per transition, with `overdue_seconds`
- **`heartbeat_recovered`** — a ping arrived on a `down` monitor, with `downtime_seconds`

A second / third / 100th event of an already-unresolved issue does **not** trigger from the ingest path — that's the job of the spike detection sweep. By design, a single shared deploy that breaks a known issue shouldn't spam the channel with every individual event.

**Delivery.** Each notifier runs in a `tokio::spawn` from the ingest path, so a slow Telegram API never blocks the SDK's request. Delivery failures are logged at `WARN` and not retried — by design, an error tracker that retries its own outbound calls amplifies outages.

**Generic webhook payload** (the same shape Telegram/Discord adapters consume). For `kind=spike` the payload additionally contains `current_hour` and `baseline_per_hour`:

```json
{
  "kind": "new_issue",
  "project_name": "Demo",
  "project_slug": "demo",
  "issue_id": 7,
  "issue_title": "TypeError: x is undefined",
  "event_count": 1,
  "level": "error",
  "environment": "production",
  "release": "1.4.2",
  "link": "http://crashbox.internal/issues/7"
}
```

```json
{
  "kind": "spike",
  "project_name": "Demo",
  "project_slug": "demo",
  "issue_id": 7,
  "issue_title": "TypeError: x is undefined",
  "event_count": 30,
  "level": "error",
  "link": "http://crashbox.internal/issues/7",
  "current_hour": 30,
  "baseline_per_hour": 0.22
}
```

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
