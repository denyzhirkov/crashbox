-- Crashbox initial schema. SQLite. Timestamps stored as ISO-8601 TEXT (UTC).

CREATE TABLE users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  email TEXT NOT NULL UNIQUE,
  name TEXT,
  password_hash TEXT NOT NULL,
  is_admin INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  user_id INTEGER NOT NULL,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

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

CREATE TABLE event_tags (
  event_id INTEGER NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  FOREIGN KEY (event_id) REFERENCES events(id) ON DELETE CASCADE
);

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

CREATE INDEX idx_events_project_received ON events(project_id, received_at DESC);
CREATE INDEX idx_events_issue_received ON events(issue_id, received_at DESC);
CREATE INDEX idx_issues_project_last_seen ON issues(project_id, last_seen DESC);
CREATE INDEX idx_issues_project_status ON issues(project_id, status);
CREATE INDEX idx_event_tags_key_value ON event_tags(key, value);
