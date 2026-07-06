# API tokens

Personal API tokens are long-lived bearer credentials for automating Crashbox — scripts, CI,
or an AI agent like Claude Code. A token grants the same access as the admin session that
minted it, is revocable in one click, and its secret is stored only as a SHA-256 hash.

## Issuing a token

In the dashboard: click your **email** in the top bar (or `⌘K → manage api tokens`) →
**new token** → name it, pick an expiry (default: never) → **mint**. The full token
(`cbx_…`, 128 random bits) is displayed **exactly once** — copy it immediately; afterwards
only the name, a short prefix, and usage dates exist anywhere.

Via API (session cookie required):

```bash
curl -b cookies.txt -X POST https://crash.example.com/api/tokens \
  -H 'content-type: application/json' \
  -d '{"name": "claude-code", "expires_in_days": 90}'   # expires_in_days optional
# → 201 { "token": "cbx_…", "id": 3, "token_prefix": "cbx_a1b2c3", … }
```

## Using a token

Send it as a bearer header — every admin endpoint accepts it exactly like a session:

```bash
curl -H "Authorization: Bearer cbx_…" https://crash.example.com/api/projects
curl -H "Authorization: Bearer cbx_…" https://crash.example.com/api/auth/me   # self-check
curl -H "Authorization: Bearer cbx_…" -X POST \
  https://crash.example.com/api/projects/1/heartbeats \
  -H 'content-type: application/json' \
  -d '{"name": "nightly-backup", "period_seconds": 86400}'
```

Typical Claude Code hand-off: mint a token named `claude-code`, then tell the agent
*"Crashbox is at https://crash.example.com, use `Authorization: Bearer cbx_…` — set up
projects and heartbeat monitors for services X, Y, Z"*. When the job is done, revoke it.

## Listing and revoking

```
GET    /api/tokens        list: name, prefix, created/expires/last_used — never the secret
DELETE /api/tokens/:id    204 — instant revocation; the next bearer request gets 401
```

`last_used_at` (refreshed lazily, ~5-minute granularity) makes forgotten tokens visible —
revoke anything that stopped being used.

## Security model

- **Storage:** only `sha256(token)` is persisted. A database leak does not leak usable
  tokens. The plaintext appears once, in the creation response, and is never logged
  (prefix only).
- **Tokens cannot manage tokens.** `/api/tokens` endpoints accept a session cookie *only* —
  a leaked token can't mint itself successors or revoke other tokens.
- **Full admin scope, deliberately.** There are no per-token scopes or roles — Crashbox is
  a single-admin instance and scoped tokens would be RBAC through the back door (out of
  scope by product guardrails). Treat a token like your password: prefer an expiry for
  hand-offs, revoke after use.
- **Expiry** is optional (`expires_in_days`, 1–3650). Expired tokens get the same uniform
  `401` as unknown ones.
- Ingestion endpoints (`/api/:id/envelope`, `/ping/:key`, `/api/:id/logs`) are unrelated to
  API tokens — they keep their own DSN-key / ping-key trust model.
