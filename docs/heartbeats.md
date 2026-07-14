# Heartbeats (dead-man's switch)

Heartbeat monitors watch things that *should* run on a schedule — cron jobs, backup scripts,
queue workers, `systemd` timers — and alert when they **stop** running. Unlike error tracking,
which needs the process to be alive enough to report, a heartbeat catches the failure modes
where nothing runs at all: the box died, cron is misconfigured, the container never started.
The signal is the *absence* of a signal.

## How it works

1. Create a monitor for a project (name + expected `period_seconds` + `grace_seconds`).
   You get a **ping URL**: `https://<crashbox>/ping/<ping_key>`.
2. Make the job hit that URL after every successful run:

   ```sh
   # end of a cron line / backup script
   pg_dump … && curl -fsS https://crash.example.com/ping/01k0abcdef… > /dev/null
   ```

3. Every ping records `last_ping_at` and keeps the monitor `up`.
4. A sweep job (every `CRASHBOX_HEARTBEAT_SWEEP_INTERVAL_SECONDS`, default 30 s) flips any
   `up` monitor to `down` once `last_ping_at + period_seconds + grace_seconds` has passed,
   and sends **one** `heartbeat_down` notification through the configured channels
   (Telegram / Discord / generic webhook — same pipeline as issue alerts).
5. The next ping flips it back to `up` and sends `heartbeat_recovered` with the downtime.

## Ping endpoint

```
GET|POST /ping/<ping_key>       (trailing slash also accepted)
```

- **Auth:** the unguessable `ping_key` alone (same trust model as DSN-key ingest). No session,
  no headers, no body — `curl <url>` is the entire client contract.
- **Responses:** `200 OK` (body `OK`) on success · `404` unknown key · `429` + `Retry-After`
  over `CRASHBOX_HEARTBEAT_MAX_PINGS_PER_MINUTE` (per monitor).
- Duplicate / early pings are fine — they just refresh `last_ping_at`.
- Deleting a monitor invalidates its URL immediately (`404`).
- The URL shown (and copied) in the UI is composed from `CRASHBOX_PUBLIC_URL` — set it to the
  address your cron jobs can actually reach, or the copy button hands out `localhost`.

## States

```
pending ──ping──▶ up ──deadline passed (sweep)──▶ down
   ▲                ◀─────────────ping────────────┘  (+ heartbeat_recovered)
   └─resume─ paused ◀──PATCH status=paused── (any state; never alerts)
```

- New monitors start `pending` and never alert until the first ping arrives.
- **Resume goes to `pending`, not `up`** — a stale `last_ping_at` from before the pause must
  not produce an instant down-alert on the next sweep tick.
- A ping on a `paused` monitor resumes it to `up`.

## Notifications

Two kinds, riding the existing notification payload (see `docs/configuration.md` →
Notifications):

| kind | fired by | extra fields |
|---|---|---|
| `heartbeat_down` | sweep, once per transition | `overdue_seconds` |
| `heartbeat_recovered` | ping endpoint | `downtime_seconds` |

Both carry `project_name`, `project_slug`, `monitor_id`, `monitor_name`, and a `link` to the
project's heartbeats page. Heartbeat payloads have no `issue_*` fields — discriminate on
`kind`, not on shape.

## Admin API

All session-authed; create/edit/delete require admin.

```
GET    /api/projects/:id/heartbeats        list (each item includes ping_url)
POST   /api/projects/:id/heartbeats        { name, description?, period_seconds, grace_seconds? = 60 }
PATCH  /api/heartbeats/:id                 { name?, description?, period_seconds?, grace_seconds?, status? }
DELETE /api/heartbeats/:id                 204; the ping URL dies with it
GET    /api/heartbeats/:id/history         status transitions, newest first (paginated)
```

`history` returns `{ "items": [{ "from_status", "to_status", "at" }, …], "total": n }` —
every flip the monitor went through (first ping, downs, recoveries, pause/resume), so an API
client can answer "how often did this fail last week" without scraping notifications. Depth
is bounded by the retention job: rows older than `CRASHBOX_RETENTION_DAYS` are pruned.

`status` accepts only `"paused"` (pause) and `"pending"` (resume) — `up`/`down` are owned by
pings and the sweep. Bounds: `period_seconds` 10 s … 30 d, `grace_seconds` 0 … 24 h.

`description` is an optional human note (≤ 500 chars) shown under the monitor name — "what
breaks if this stops". On PATCH it is three-state: omit the field to keep the note, send a
blank string to clear it, send text to replace it.

## Declarative provisioning (env)

For IaC-style deployments, monitors can be provisioned at startup instead of through the UI:

```bash
CRASHBOX_HEARTBEAT_MONITORS='[
  {"name":"db-backup","ping_key":"k7f3xxxxxxxxxxxxxxxxxxxxxx","period_seconds":86400,"grace_seconds":3600,"description":"nightly pg_dump"},
  {"name":"queue-worker","ping_key":"q9m1xxxxxxxxxxxxxxxxxxxxxx","period_seconds":60}
]'
```

Applied idempotently on every startup, against the lowest-id (default) project:

- `name` is the identity: no monitor with that name → created; exists → converged.
- The operator supplies the `ping_key` (16–128 chars of `[A-Za-z0-9_-]`), so ping URLs
  survive container recreation. It is the only authentication on the URL — generate it like
  a secret (`openssl rand -hex 16`). Declaring a new key rotates it.
- Env wins for what it declares: `ping_key` and `period_seconds` always, `grace_seconds` and
  `description` only when present in the entry — otherwise UI edits survive restarts.
- Monitors *not* listed in the env are never touched or deleted, and provisioning never
  changes `status` or transition history.
- A malformed entry (bad JSON, short key, out-of-bounds period, duplicate name/key) fails
  startup loud — a silently-skipped monitor is a dead-man's switch that never arms.

## Metrics

- `crashbox_heartbeat_pings_total` — accepted pings
- `crashbox_heartbeat_transitions_total{to="down"|"up"}` — sweep down-flips and recoveries

## Limitations (by design, MVP)

- **`pending` never times out.** A monitor that has never pinged has no deadline to be late
  against; it sits `pending` until its first ping. Check the UI after wiring up a new cron.
- **Down-alert fires once per transition.** There are no repeated "still down" reminders.
- **Sweep granularity is the sweep interval** (default 30 s) — a monitor is marked down up to
  one interval after its deadline, not at the exact second.
- **No ping history.** Only `last_ping_at` is stored; there is no per-ping log or uptime
  percentage.
- **Passive only.** Crashbox never probes your services (no outbound HTTP checks) — that
  needs an external vantage point and is a different tool.
- **Fixed period only.** No cron-expression schedules; use the interval + grace that covers
  your schedule's worst case.
