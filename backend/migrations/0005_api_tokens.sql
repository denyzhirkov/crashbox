-- Personal API tokens: long-lived bearer credentials for automation (scripts, Claude Code).
-- Only the SHA-256 of the token is stored; the plaintext is shown once at creation.
CREATE TABLE api_tokens (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  token_prefix TEXT NOT NULL, -- first chars of the plaintext, for identification in lists
  created_at TEXT NOT NULL,
  expires_at TEXT,            -- NULL = never expires
  last_used_at TEXT,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_api_tokens_user ON api_tokens(user_id);
