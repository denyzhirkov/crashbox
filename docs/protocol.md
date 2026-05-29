# Sentry ingest protocol — what Crashbox supports

Crashbox implements a **practical subset** of Sentry's HTTP ingestion. The goal is for unmodified
official Sentry SDKs to send events to Crashbox by changing only the DSN.

Crashbox is not a full Sentry replacement. This document is the single source of truth for
what we do — and don't — accept.

---

## Endpoints

| Method | Path | Purpose | Status |
|---|---|---|---|
| POST | `/api/:project_id/envelope` | SDK envelope upload | ✅ supported |
| POST | `/api/:project_id/envelope/` | trailing-slash variant | ✅ supported |
| POST | `/api/:project_id/store/` | legacy non-envelope endpoint | ❌ not in MVP |

The envelope endpoint matches the path SDKs derive from a DSN of the form
`http(s)://<public_key>@<host>[:port]/<project_id>`.

### Authentication

The `sentry_key` value must equal the project's `public_key` and the project must match the
`:project_id` in the URL. We accept either source the SDKs use:

1. `X-Sentry-Auth: Sentry sentry_version=7, sentry_key=PUBLIC_KEY, sentry_client=...`
2. `?sentry_key=PUBLIC_KEY` query string

Other `X-Sentry-Auth` fields (`sentry_version`, `sentry_client`, etc.) are accepted but ignored.

Failure modes:

- Missing `sentry_key` → `401 unauthorized`
- Unknown `sentry_key` → `401 unauthorized`
- `sentry_key` doesn't match `:project_id` path segment → `401 unauthorized`

---

## Envelope format

We parse Sentry's line-oriented envelope format:

```
{envelope_header_json}\n
{item_header_json}\n
{item_payload}\n
{item_header_json}\n
{item_payload}\n
...
```

- Envelope header is parsed as JSON. We use `event_id` if present (as a fallback when the event
  payload omits it) but do not validate other fields.
- Each item header carries at least `type`. If `length` is present, the payload is exactly that
  many bytes; otherwise the payload runs until the next `\n`.

### Item types

| `type` | Behavior |
|---|---|
| `event` | Parsed, normalized, stored, grouped into an issue |
| anything else | Skipped (or stored as raw with `CRASHBOX_STORE_RAW_UNSUPPORTED_ITEMS=true`, not yet implemented in MVP) |

**MVP only processes the first `event` item per envelope.** Most SDKs send one event per
envelope; if more arrive only the first is recorded. Future versions may relax this.

### Failure modes

- Body > `CRASHBOX_MAX_ENVELOPE_BYTES` → `413 payload too large`
- Event payload > `CRASHBOX_MAX_EVENT_BYTES` → `413 payload too large`
- Unparseable envelope header → `400 bad request`
- Invalid JSON in an item header → `400 bad request` (with byte offset in the message)
- Item header declares a `length` past end of buffer → `400 bad request`
- Item payload is not valid UTF-8 → `400 bad request`
- Item payload is not valid JSON (for `type=event`) → `400 bad request`

The server **never panics** on malformed envelopes — that's a hard invariant enforced by tests.

---

## Event fields extracted

We extract the following fields from each event JSON payload and persist them in indexed columns
on the `events` table:

| Field | Source path in payload |
|---|---|
| `event_id` | `event_id` (envelope header is fallback) |
| `timestamp` | `timestamp` (ISO-8601 or Unix seconds) |
| `platform` | `platform` |
| `level` | `level` (defaults to `error` when an exception is present, `info` otherwise) |
| `logger` | `logger` |
| `transaction_name` | `transaction` |
| `environment` | `environment` |
| `release` | `release` |
| `server_name` | `server_name` |
| `message` | `message` (string or `{formatted, message}` object) |
| `exception_type` | `exception.values[last].type` |
| `exception_value` | `exception.values[last].value` |
| `request_url` | `request.url` |
| `user_id` | `user.id` |
| `user_email` | `user.email` |
| `culprit` | `culprit` |
| `tags` | `tags` (object or array of pairs) → `event_tags` table |
| `breadcrumbs` | `breadcrumbs.values` or bare `breadcrumbs[]` → `event_breadcrumbs` table |
| `fingerprint` | `fingerprint` (used by grouping, see below) |
| `raw_json` | the entire original event JSON, stored verbatim |

Limits to prevent payload bloat:

- Tags: first 100 per event are stored.
- Breadcrumbs: first 200 per event are stored.

If a field is missing or in an unexpected shape, the column is left `NULL`. **Ingestion never
fails because of an unrecognized field**.

---

## Grouping (fingerprinting)

Crashbox uses its own deterministic lightweight grouping. **We do not try to bit-match Sentry's
grouping algorithm**; that's a significant body of code with cross-version compatibility quirks
that aren't worth carrying.

Order of preference (first non-empty wins):

1. **Custom fingerprint.** If the event includes `"fingerprint": [...]` and it isn't only the
   placeholder `["{{ default }}"]`, we hash `custom|<platform>|<joined-parts>`.
2. **Exception.** Hash of `exception|<platform>|<type>|<normalized-value>|<top-frame-sig>`.
   - `top-frame-sig` is the topmost `in_app: true` frame; otherwise the topmost frame at all.
   - The frame signature is `function@module:filename:lineno`.
3. **Message.** Hash of `message|<platform>|<normalized-message>`.
4. **Fallback.** Hash of `fallback|<platform>|<transaction|logger|event_id>`.

### Message normalization

To make grouping stable across requests with variable IDs, we substitute:

- UUIDs (8-4-4-4-12 hex) → `<uuid>`
- Long hex strings (≥16 chars, all hex) → `<hex>`
- Pure long numbers (≥8 digits) → `<num>`
- Whitespace runs → single space
- Length is capped at 500 chars

Example: `"row 11111111 missing"` and `"row 22222222 missing"` produce the same fingerprint.
`"row 1 missing"` and `"row 2 missing"` do **not** (single-digit numbers stay literal).

Hash algorithm: SHA-1, hex-encoded (160 bits). Stored as `issues.fingerprint`.

### Issue title

The title shown in the UI is built from the event:

1. `<exception_type>: <exception_value>`
2. else `message`
3. else `transaction_name`
4. else `<unknown event>`

Title is set on first occurrence of an issue and not changed afterwards.

### Auto-reopen

If an event lands on an issue whose status is `resolved`, the status flips back to `unresolved`
in the same transaction as the event insert. A `reopened` notification fires (see
`docs/configuration.md` → Notifications).

---

## What we do NOT implement

This is intentional MVP scope, not bugs:

- **Performance traces / spans / transactions** — `type=transaction` items are skipped.
- **Session replay** — `type=replay_*` items are skipped.
- **Source maps** — stack frames are shown as the SDK sent them; we do not fetch or apply maps.
- **Attachments** — `type=attachment` items are skipped.
- **Profiles** — `type=profile` items are skipped.
- **Crons / monitor check-ins** — `type=check_in` items are skipped.
- **Release Health / sessions** — `type=session` items are skipped.
- **Discover / advanced query DSL** — only the simple filter set on `/api/projects/:id/issues` is
  supported (status, level, environment, release, query, limit, offset).
- **Organizations, teams, RBAC** — single tenant. Admin/user distinction only.
- **Webhooks / notifications** — config vars exist (`CRASHBOX_TELEGRAM_*`, etc.) but
  notifications are post-MVP.

If your code depends on any of the above, Crashbox is not the right tool.

---

## Where the fixtures live

Real envelopes captured from official SDKs live under
[`backend/tests/fixtures/envelopes/`](../backend/tests/fixtures/envelopes/). They are stored as
the event JSON the SDK posts inside the envelope, suitable for replay in integration tests.

Current fixtures:

- `sentry-node-typeerror.event.json` — `@sentry/node` v8.55.2, `TypeError` from a real Node script.
