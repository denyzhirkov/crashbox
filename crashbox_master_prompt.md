# Crashbox — Master Prompt for IDE Agent

## 0. Role and working mode

You are an experienced product-minded full-stack engineer and architect. Your task is to build **Crashbox**: a minimal, self-hosted, Sentry-compatible error tracking server with a lightweight UI.

Work as an autonomous coding agent inside this repository. Do not over-engineer. Prefer small, working vertical slices. Every feature must be useful for a tiny production project and must keep the final Docker image and runtime resource usage low.

Primary goal:

> Build a tiny Sentry-compatible error inbox that existing backend and frontend Sentry SDKs can send events to by changing only the DSN.

The product is not intended to match full Sentry feature parity. It should provide the most valuable 20% of Sentry-like behavior with the smallest possible operational footprint.

Project name: **Crashbox**.

---

## 1. Product vision

Crashbox is a small self-hosted service for developers who want Sentry-like error tracking without running the full Sentry stack.

The ideal user should be able to run:

```bash
docker run \
  -p 8080:8080 \
  -v crashbox-data:/data \
  -e CRASHBOX_ADMIN_EMAIL=admin@example.com \
  -e CRASHBOX_ADMIN_PASSWORD=change-me \
  -e CRASHBOX_PROJECT_NAME=my-app \
  ghcr.io/<owner>/crashbox:latest
```

Then use a DSN like:

```text
http://PUBLIC_KEY@localhost:8080/1
```

And configure a normal Sentry SDK like:

```ts
Sentry.init({
  dsn: "http://PUBLIC_KEY@localhost:8080/1",
});
```

No custom SDK. No custom client protocol. No vendor lock-in.

---

## 2. Hard product constraints

These constraints are more important than feature count.

1. **Single container first**  
   Crashbox must run as one Docker container by default.

2. **Low memory usage**  
   Target idle memory should be as low as reasonably possible. Avoid large background workers, heavy queues, JVM/Node backend services, or analytics engines.

3. **SQLite first**  
   Default storage is a local SQLite database in `/data/crashbox.db`. Optional PostgreSQL support may be added later, but must not be required for MVP.

4. **Existing Sentry SDK compatibility**  
   For MVP, focus on compatibility with official Sentry SDKs for:
   - browser/frontend JavaScript
   - Node.js backend
   - optionally Python or Rust later

5. **Only DSN should change**  
   The user should not have to change application code except replacing the Sentry DSN.

6. **No full Sentry clone**  
   Do not implement organizations, teams, complex RBAC, performance monitoring, session replay, cron monitors, release health, Discover, Snuba-like analytics, or source map processing in MVP.

7. **Tiny but nice UI**  
   UI should be fast, clean, understandable, and useful. SolidJS + TailwindCSS is the chosen frontend stack.

---

## 3. Tech stack

### Backend

Use:

- Rust
- Axum
- Tokio
- SQLx
- SQLite
- Serde / serde_json
- tracing / tracing-subscriber
- tower-http
- argon2 or another safe password hashing crate
- uuid or ulid where useful
- chrono or time crate

Optional later:

- Postgres support via SQLx feature flags
- zstd/gzip decompression
- OpenTelemetry ingestion bridge

### Frontend

Use:

- SolidJS
- TypeScript
- Vite
- TailwindCSS
- simple fetch API wrapper or TanStack Query if it does not bloat the app too much

### Packaging

Target:

- one Rust binary serving API and static frontend assets
- Docker image with minimal runtime base
- `/data` volume for database and optional attachments

---

## 4. Important external protocol facts

Crashbox should implement the practical subset of Sentry ingestion.

Sentry SDKs send events using **envelopes**. The main ingestion endpoint is:

```text
POST /api/{PROJECT_ID}/envelope/
```

Crashbox must support at least this endpoint.

Envelope format is line-oriented:

```text
{envelope headers JSON}\n
{item headers JSON}\n
{item payload}\n
{item headers JSON}\n
{item payload}\n
...
```

Important envelope/item concepts:

- Envelope header may include `dsn`, `sent_at`, `sdk`, `trace`, etc.
- Item header includes a `type` field.
- The important MVP item type is `event`.
- Other item types should not crash the server. Unsupported item types should be ignored or stored as raw unsupported items depending on configuration.
- Event payload is JSON.

Expected SDK endpoint behavior:

```text
DSN: http://PUBLIC_KEY@localhost:8080/1
SDK sends to: http://localhost:8080/api/1/envelope/
```

Also consider trailing slash variants:

```text
/api/1/envelope
/api/1/envelope/
```

Crashbox should accept both.

Legacy endpoint support:

```text
POST /api/{PROJECT_ID}/store/
```

This endpoint may be added as optional compatibility, but envelope support is the priority.

---

## 5. MVP definition

Build the smallest useful Crashbox.

### MVP must include

1. Project bootstrap from environment variables
2. Admin user bootstrap from environment variables
3. Project DSN generation
4. Sentry envelope endpoint
5. Event parsing and storage
6. Basic issue grouping
7. Issue list UI
8. Issue detail UI
9. Raw JSON event viewer
10. Resolve/unresolve issue
11. Basic retention cleanup
12. Docker image
13. Minimal documentation

### MVP should not include

- advanced Sentry API compatibility
- source maps
- performance traces
- session replay
- attachments
- user/team roles
- organizations
- billing
- external queue systems
- distributed deployment
- ClickHouse or Kafka
- complex search engine

---

## 6. Runtime configuration

Crashbox should be highly configurable at container startup using environment variables and/or CLI flags.

Environment variables should have prefix `CRASHBOX_`.

### Core server config

```env
CRASHBOX_HOST=0.0.0.0
CRASHBOX_PORT=8080
CRASHBOX_PUBLIC_URL=http://localhost:8080
CRASHBOX_DATABASE_URL=sqlite:///data/crashbox.db
CRASHBOX_DATA_DIR=/data
CRASHBOX_LOG_LEVEL=info
CRASHBOX_SECRET_KEY=change-me-generate-random
```

### Admin bootstrap

```env
CRASHBOX_ADMIN_EMAIL=admin@example.com
CRASHBOX_ADMIN_PASSWORD=change-me
CRASHBOX_ADMIN_NAME=Admin
```

Rules:

- On first boot, create admin user if no users exist.
- If users already exist, do not overwrite password unless an explicit reset flag is provided.
- Never log plaintext passwords.

Optional explicit reset variable:

```env
CRASHBOX_FORCE_ADMIN_RESET=false
```

### Project bootstrap

```env
CRASHBOX_PROJECT_NAME=my-app
CRASHBOX_PROJECT_PLATFORM=javascript
CRASHBOX_PROJECT_ENVIRONMENT=production
CRASHBOX_PROJECT_PUBLIC_KEY=
CRASHBOX_PROJECT_SECRET_KEY=
```

Rules:

- On first boot, create default project if no projects exist.
- Generate public key if not provided.
- Generate secret key if not provided.
- Show generated DSN in logs once during bootstrap, but avoid leaking secret values unnecessarily.

### Ingestion limits

```env
CRASHBOX_MAX_ENVELOPE_BYTES=1048576
CRASHBOX_MAX_EVENT_BYTES=524288
CRASHBOX_MAX_EVENTS_PER_MINUTE_PER_PROJECT=600
CRASHBOX_ACCEPT_UNKNOWN_ITEM_TYPES=false
CRASHBOX_STORE_RAW_UNSUPPORTED_ITEMS=false
CRASHBOX_ENABLE_LEGACY_STORE_ENDPOINT=false
```

### Retention

```env
CRASHBOX_RETENTION_DAYS=30
CRASHBOX_MAX_EVENTS_PER_ISSUE=100
CRASHBOX_CLEANUP_INTERVAL_SECONDS=3600
```

Behavior:

- Keep issue summaries longer than raw events.
- Delete old events according to retention.
- Optionally keep last N events per issue even if old.

### UI config

```env
CRASHBOX_UI_ENABLED=true
CRASHBOX_UI_APP_NAME=Crashbox
CRASHBOX_UI_THEME=system
```

### Security config

```env
CRASHBOX_COOKIE_SECURE=false
CRASHBOX_CORS_ALLOWED_ORIGINS=*
CRASHBOX_TRUST_PROXY_HEADERS=false
CRASHBOX_ALLOW_PUBLIC_SIGNUP=false
```

MVP may keep auth simple, but do not leave admin APIs open.

### Notifications, optional later

```env
CRASHBOX_TELEGRAM_BOT_TOKEN=
CRASHBOX_TELEGRAM_CHAT_ID=
CRASHBOX_DISCORD_WEBHOOK_URL=
CRASHBOX_GENERIC_WEBHOOK_URL=
```

---

## 7. Suggested repository structure

```text
crashbox/
├─ backend/
│  ├─ Cargo.toml
│  ├─ migrations/
│  └─ src/
│     ├─ main.rs
│     ├─ config.rs
│     ├─ app_state.rs
│     ├─ http/
│     │  ├─ mod.rs
│     │  ├─ routes.rs
│     │  ├─ auth.rs
│     │  ├─ ingest.rs
│     │  ├─ projects.rs
│     │  ├─ issues.rs
│     │  └─ health.rs
│     ├─ sentry/
│     │  ├─ mod.rs
│     │  ├─ dsn.rs
│     │  ├─ envelope.rs
│     │  ├─ event.rs
│     │  ├─ normalize.rs
│     │  └─ grouping.rs
│     ├─ db/
│     │  ├─ mod.rs
│     │  ├─ users.rs
│     │  ├─ projects.rs
│     │  ├─ events.rs
│     │  └─ issues.rs
│     ├─ security/
│     │  ├─ mod.rs
│     │  ├─ password.rs
│     │  └─ sessions.rs
│     └─ jobs/
│        ├─ mod.rs
│        └─ cleanup.rs
├─ frontend/
│  ├─ package.json
│  ├─ vite.config.ts
│  ├─ tailwind.config.ts
│  └─ src/
│     ├─ main.tsx
│     ├─ App.tsx
│     ├─ api/
│     ├─ pages/
│     ├─ components/
│     └─ styles.css
├─ Dockerfile
├─ docker-compose.yml
├─ README.md
└─ docs/
   ├─ protocol.md
   ├─ configuration.md
   └─ development.md
```

If a monorepo layout becomes inconvenient, adjust it, but keep the project easy to understand.

---

## 8. Database model

Use SQLite migrations.

### users

```sql
CREATE TABLE users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  email TEXT NOT NULL UNIQUE,
  name TEXT,
  password_hash TEXT NOT NULL,
  is_admin BOOLEAN NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

### sessions

```sql
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
```

### projects

```sql
CREATE TABLE projects (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  slug TEXT NOT NULL UNIQUE,
  platform TEXT,
  default_environment TEXT,
  public_key TEXT NOT NULL UNIQUE,
  secret_key_hash TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

### issues

```sql
CREATE TABLE issues (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL,
  fingerprint TEXT NOT NULL,
  title TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'unresolved',
  level TEXT,
  platform TEXT,
  first_seen TEXT NOT NULL,
  last_seen TEXT NOT NULL,
  event_count INTEGER NOT NULL DEFAULT 0,
  last_event_id INTEGER,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE(project_id, fingerprint),
  FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);
```

### events

```sql
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT,
  project_id INTEGER NOT NULL,
  issue_id INTEGER,
  timestamp TEXT,
  received_at TEXT NOT NULL,
  level TEXT,
  platform TEXT,
  environment TEXT,
  release TEXT,
  logger TEXT,
  transaction_name TEXT,
  message TEXT,
  exception_type TEXT,
  exception_value TEXT,
  culprit TEXT,
  server_name TEXT,
  request_url TEXT,
  user_id TEXT,
  user_email TEXT,
  fingerprint TEXT,
  raw_json TEXT NOT NULL,
  FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
  FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE SET NULL
);
```

### event_tags

```sql
CREATE TABLE event_tags (
  event_id INTEGER NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  FOREIGN KEY (event_id) REFERENCES events(id) ON DELETE CASCADE
);
```

### event_breadcrumbs

```sql
CREATE TABLE event_breadcrumbs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id INTEGER NOT NULL,
  timestamp TEXT,
  category TEXT,
  level TEXT,
  message TEXT,
  data_json TEXT,
  FOREIGN KEY (event_id) REFERENCES events(id) ON DELETE CASCADE
);
```

Add indexes:

```sql
CREATE INDEX idx_events_project_received ON events(project_id, received_at DESC);
CREATE INDEX idx_events_issue_received ON events(issue_id, received_at DESC);
CREATE INDEX idx_issues_project_last_seen ON issues(project_id, last_seen DESC);
CREATE INDEX idx_issues_project_status ON issues(project_id, status);
CREATE INDEX idx_event_tags_key_value ON event_tags(key, value);
```

---

## 9. Sentry event parsing priorities

When receiving an event payload, extract these fields if present:

```text
event_id
timestamp
platform
level
logger
transaction
environment
release
server_name
message
exception.values[0].type
exception.values[0].value
exception.values[0].stacktrace.frames
threads.values
request.url
request.method
request.headers
user.id
user.email
tags
breadcrumbs
extra
fingerprint
```

Always store the original raw event JSON.

If parsing fails:

- return a clear 400 only if the payload is invalid for the accepted endpoint
- avoid panics
- include internal tracing logs

Unsupported but valid envelope item types should not make the whole envelope fail in MVP. Ignore them safely.

---

## 10. Issue grouping algorithm

MVP grouping should be simple but stable.

Suggested algorithm:

1. If event has explicit `fingerprint`, use it.
2. Else if exception exists:
   - use exception type
   - use exception value normalized
   - use top in-app stack frame if detectable
   - fallback to top stack frame
3. Else if message exists:
   - use normalized message
4. Else:
   - use event type + platform + transaction or logger

Normalize message by:

- replacing UUIDs with `<uuid>`
- replacing long hex strings with `<hex>`
- replacing numbers with `<num>` only when useful
- trimming whitespace
- limiting length

Pseudo-code:

```rust
fn fingerprint(event: &NormalizedEvent) -> String {
    if let Some(custom) = event.custom_fingerprint() {
        return sha1(custom.join("|"));
    }

    if let Some(exception) = event.primary_exception() {
        return sha1(format!(
            "exception|{}|{}|{}|{}",
            event.platform,
            exception.ty,
            normalize_message(exception.value),
            best_stack_frame_signature(exception),
        ));
    }

    if let Some(message) = &event.message {
        return sha1(format!(
            "message|{}|{}",
            event.platform,
            normalize_message(message),
        ));
    }

    sha1(format!("fallback|{}|{}", event.platform, event.event_id))
}
```

Do not try to perfectly copy Sentry grouping in MVP. Document that Crashbox uses its own lightweight grouping.

---

## 11. HTTP API

### Public ingestion API

```text
POST /api/:project_id/envelope/
POST /api/:project_id/envelope
```

Optional later:

```text
POST /api/:project_id/store/
```

### Health

```text
GET /healthz
GET /readyz
```

### Auth

```text
POST /api/auth/login
POST /api/auth/logout
GET  /api/auth/me
```

### Projects

```text
GET  /api/projects
POST /api/projects
GET  /api/projects/:id
PATCH /api/projects/:id
GET  /api/projects/:id/dsn
POST /api/projects/:id/rotate-key
```

### Issues

```text
GET   /api/projects/:project_id/issues
GET   /api/issues/:id
PATCH /api/issues/:id
GET   /api/issues/:id/events
GET   /api/events/:id
```

Issue filters:

```text
status=unresolved|resolved|all
level=error|warning|info
environment=production
release=1.0.0
query=TypeError
limit=50
offset=0
```

---

## 12. UI requirements

The UI should feel simple and fast.

### Pages

#### Login

- email
- password
- submit

#### Projects

- list projects
- project name
- platform
- DSN copy button
- latest issue count

#### Issues list

Columns:

- status
- level
- title
- event count
- environment
- release
- last seen

Filters:

- status
- environment
- release
- level
- text search

#### Issue detail

Show:

- title
- status
- resolve/unresolve button
- event count
- first seen
- last seen
- level
- platform
- environment
- release
- exception type/value
- stacktrace
- breadcrumbs
- tags
- user
- request
- raw JSON

#### Settings

- project DSN
- public key
- rotate key
- retention settings, if implemented

### UI style

- dark/light/system theme if easy
- TailwindCSS
- readable stacktrace blocks
- monospaced raw JSON viewer
- avoid heavy component libraries unless needed

---

## 13. Docker and deployment

The final Docker image should:

- build frontend
- build Rust backend release binary
- copy frontend static assets into backend or serve from embedded directory
- run as non-root user if possible
- expose port 8080
- use `/data` as persistent volume

Example Docker Compose:

```yaml
services:
  crashbox:
    image: ghcr.io/<owner>/crashbox:latest
    ports:
      - "8080:8080"
    volumes:
      - crashbox-data:/data
    environment:
      CRASHBOX_PUBLIC_URL: "http://localhost:8080"
      CRASHBOX_ADMIN_EMAIL: "admin@example.com"
      CRASHBOX_ADMIN_PASSWORD: "change-me"
      CRASHBOX_PROJECT_NAME: "my-app"

volumes:
  crashbox-data:
```

---

## 14. Compatibility testing

Create a local test script or example apps for:

### Browser JavaScript

```ts
import * as Sentry from "@sentry/browser";

Sentry.init({
  dsn: "http://PUBLIC_KEY@localhost:8080/1",
});

Sentry.captureException(new Error("Crashbox browser test"));
```

### Node.js

```ts
import * as Sentry from "@sentry/node";

Sentry.init({
  dsn: "http://PUBLIC_KEY@localhost:8080/1",
});

Sentry.captureException(new Error("Crashbox node test"));
await Sentry.flush(2000);
```

Acceptance criteria:

- SDK sends request to Crashbox without client code changes except DSN
- Crashbox responds successfully
- Event appears in UI
- Repeated same error groups into same issue
- Different errors create different issues
- Raw JSON is available

---

## 15. Security notes

Crashbox is small but should not be careless.

Implement:

- password hashing, not plaintext passwords
- signed or server-side sessions
- ingestion body size limits
- per-project rate limiting
- safe JSON parsing
- no panic on malformed envelopes
- no plaintext secrets in logs
- CORS config
- secure cookie option

Do not implement public signup by default.

---

## 16. Development strategy

Build in vertical slices.

### Slice 1 — bootable backend

- Axum server
- config loader
- SQLite connection
- migrations
- `/healthz`
- static frontend placeholder

### Slice 2 — bootstrap

- create admin user from env
- create default project from env
- generate DSN
- log DSN on first startup

### Slice 3 — ingestion

- implement `/api/:project_id/envelope/`
- parse envelope minimally
- extract event item
- store raw event

### Slice 4 — grouping

- normalize event
- create/update issue
- link event to issue

### Slice 5 — UI

- login
- project list
- issue list
- issue detail
- raw JSON viewer

### Slice 6 — production basics

- Dockerfile
- docker-compose
- retention cleanup
- rate limiting
- README

---

## 17. Acceptance criteria for first public release

Crashbox is ready for a first public alpha when:

1. It can run via Docker with one command.
2. It creates an admin account from env vars.
3. It creates a default project from env vars.
4. It displays/copies the project DSN.
5. A standard Sentry JavaScript browser SDK can send an error by changing only DSN.
6. A standard Sentry Node.js SDK can send an error by changing only DSN.
7. Events appear in UI.
8. Similar events are grouped into issues.
9. Issues can be resolved/unresolved.
10. Old events can be cleaned up by retention.
11. The service survives malformed envelopes without crashing.
12. README explains setup clearly.

---

## 18. README positioning text

Use this positioning:

> Crashbox is a tiny self-hosted Sentry-compatible error tracking server for small projects. It accepts events from existing Sentry SDKs by changing only the DSN, stores them locally, groups them into issues, and gives you a simple web UI to inspect crashes.

Do not claim full Sentry compatibility. Say:

> Crashbox implements a practical subset of Sentry ingestion focused on error events. It is not a full Sentry replacement and does not implement every Sentry feature.

---

## 19. Important implementation philosophy

When choosing between two solutions, prefer:

- simpler over more powerful
- one binary over many services
- SQLite over external infrastructure
- raw JSON preservation over premature normalization
- clear UI over advanced analytics
- documented limitations over fake compatibility
- stable MVP over incomplete feature sprawl

The project should feel like:

> “I need error tracking for my small app tonight.”

Not:

> “I need to operate an observability platform.”

---

## 20. First task for the coding agent

Start by creating the repository skeleton and implementing the first vertical slice:

1. Rust Axum backend
2. config loader from env
3. SQLite database with migrations
4. `/healthz` endpoint
5. bootstrap admin user
6. bootstrap default project
7. generate and print DSN
8. minimal README with Docker/local development instructions

After that, implement the envelope ingestion endpoint and verify it using a tiny Node.js script with `@sentry/node`.
