# Live Logs

Crashbox has two independent channels:

- **Events** — errors and crashes, parsed, grouped into issues, and **persisted** in SQLite.
- **Live Logs** — ordinary application logs ("what's happening right now"), streamed in real time and **never written to disk**.

Live Logs is for tailing: you open a project's log view, watch lines arrive, filter/group them, and close it. There is no history search, no alerting, and no storage beyond a small in-memory ring buffer. It is deliberately lossy and ephemeral.

This is a Crashbox extension, not part of the Sentry protocol — though the Sentry `log` envelope item is accepted for convenience (see below).

---

## Architecture

```
                    POST /api/:id/logs            ┌──────────────────────────┐
  your app  ───────────────────────────────────▶ │  LiveLogHub (per project)│
            (or Sentry `log` envelope item)       │                          │
                                                  │  ring buffer (RAM, last  │
                                                  │   N lines — scrollback)  │
  browser  ◀── GET /api/projects/:id/logs/stream ─│  broadcast (live, lossy) │
                  (SSE, session-authed)            └──────────────────────────┘
```

- The ring buffer holds the last `CRASHBOX_LIVE_LOG_BUFFER_PER_PROJECT` lines per project so a freshly-opened stream gets immediate scrollback. It lives in RAM and evaporates on restart.
- Live fan-out uses a broadcast channel. A subscriber that can't keep up is **dropped** (lines are skipped); it is a live tail, not a guaranteed log.

---

## Ingesting logs

Two accepted formats, both authenticated by the **DSN public key** (same credential your SDK already uses for events — only the DSN changes, never your app code beyond pointing logs at Crashbox).

### 1. Dedicated endpoint (recommended)

```
POST /api/:project_id/logs
Authorization: via DSN public key — send it as either
  X-Sentry-Auth: Sentry sentry_key=<public_key>
  or  ?sentry_key=<public_key>
```

Body may be a **JSON array**, a **single JSON object**, or **newline-delimited JSON (NDJSON)**:

```jsonc
// a single line
{ "level": "info", "message": "user signed in", "logger": "auth", "ts": "2026-06-02T10:00:00Z", "attrs": { "user_id": 42 } }
```

Fields (all optional except that the value must be a JSON object):

| Field | Meaning |
|---|---|
| `level` | `trace` \| `debug` \| `info` \| `warn` \| `error` \| `fatal`. Unknown / missing → `info`. |
| `message` (aliases `msg`, `body`) | The log line. Truncated to `CRASHBOX_LIVE_LOG_MESSAGE_MAX_BYTES`. |
| `logger` (alias `source`) | Logger / subsystem name. |
| `ts` (alias `timestamp`) | ISO-8601 string or epoch-seconds number. Missing → server receive time. |
| `trace_id` | Optional correlation id. |
| any other key | Collected into `attrs` (structured fields). |

Response — `202 Accepted`:

```json
{ "accepted": 2, "skipped": 0 }
```

Malformed entries are **skipped, not fatal**: a bad line in an NDJSON batch never rejects the rest.

`curl` example:

```bash
curl -X POST "http://localhost:8080/api/1/logs" \
  -H "X-Sentry-Auth: Sentry sentry_key=YOUR_PUBLIC_KEY" \
  -H "content-type: application/x-ndjson" \
  --data-binary $'{"level":"info","message":"boot"}\n{"level":"warn","message":"slow query","logger":"db"}\n'
```

### 2. Sentry `log` envelope item

If your SDK emits Sentry structured logs, they arrive as a `log` item inside the normal envelope at `POST /api/:project_id/envelope/`. Crashbox detects `"type":"log"` items and feeds them to Live Logs instead of the event store. The OTel-style typed attributes (`{"value": x, "type": "string"}`) are unwrapped to their bare values. Event items in the same envelope are processed as usual.

---

## Streaming logs (UI / SSE)

```
GET /api/projects/:id/logs/stream
Authorization: admin session cookie (same as the rest of the dashboard API)
```

A [Server-Sent Events](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events) stream. On connect it replays the ring-buffer snapshot, then streams live lines. Each event's `data:` is one `LogRecord` as JSON. Heartbeat comments keep proxies from timing the connection out; the browser `EventSource` reconnects automatically.

Optional query filters (applied server-side to reduce bandwidth, AND-combined):

| Param | Effect |
|---|---|
| `level` | Minimum severity floor (e.g. `level=warn` drops trace/debug/info). |
| `logger` | Case-insensitive substring match on `logger`. |
| `q` | Case-insensitive substring match on `message`. |

The dashboard adds further **client-side** conveniences on top of the live stream: a severity floor, free-text search across message/logger/attrs, pause/resume with a buffered "N new lines" pill, group-by-logger, a 60-second throughput sparkline, and auto-scroll. The client keeps at most 1000 lines in memory.

## Fetching a snapshot (no stream)

```
GET /api/projects/:id/logs/recent?level=…&logger=…&q=…&limit=…
```

One-shot copy of the current ring buffer as `{ "items": […], "count": n }`, oldest first —
the same filters as the stream, plus `limit` to keep only the newest N after filtering.
Built for API clients (scripts, agents) that want "what's in the logs right now" as a single
request instead of opening an SSE connection and cutting it off. Same session/bearer auth as
the stream; remember the buffer is RAM-only, so this is a snapshot, not history.

---

## Limitations (by design)

- **Not persisted.** Logs live only in RAM. A restart clears everything; there is no history beyond the ring buffer.
- **Single-instance.** Logs are visible only on the Crashbox instance that received them. There is no cross-instance fan-out (consistent with the single-container deployment model). If you run multiple replicas behind a load balancer, a browser sees only the replica its stream landed on.
- **Lossy.** Under load, or for a slow client, lines are dropped rather than queued.
- **No alerting / no correlation.** Live Logs does not raise notifications and is not linked to issues.

If you need durable, searchable, multi-instance logs, use a dedicated log aggregator — Live Logs intentionally stays a lightweight real-time tail.
