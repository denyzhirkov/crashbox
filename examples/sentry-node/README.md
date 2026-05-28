# @sentry/node → Crashbox smoke test

Sends one real exception through the official `@sentry/node` SDK to a running Crashbox.

## Run

```bash
# 1. Start Crashbox somewhere (see project README).
# 2. Get the DSN from logs or the UI.
# 3.
cd examples/sentry-node
pnpm install      # or: npm install
DSN=http://<PUBLIC_KEY>@localhost:8080/1 node send.js
```

The SDK is **unmodified** — only the DSN changes from whatever you used with hosted Sentry.

## What it proves

- Crashbox accepts the envelope shape the official SDK produces.
- The event ends up in the issues list with the right exception type and stacktrace.
- Repeated runs group into the same issue (same `TypeError: Cannot read properties of null (reading 'name')`).
