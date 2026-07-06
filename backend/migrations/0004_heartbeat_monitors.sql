-- Heartbeat monitors (dead-man's switch): a cron job or service pings us on a fixed
-- period; silence past period + grace flips the monitor to 'down' and fires an alert.
-- No ping history is kept — last_ping_at is the whole state.
CREATE TABLE heartbeat_monitors (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL,
  name TEXT NOT NULL,
  ping_key TEXT NOT NULL UNIQUE,
  period_seconds INTEGER NOT NULL,
  grace_seconds INTEGER NOT NULL DEFAULT 60,
  status TEXT NOT NULL DEFAULT 'pending', -- pending | up | down | paused
  last_ping_at TEXT,
  last_transition_at TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX idx_heartbeat_monitors_project ON heartbeat_monitors(project_id);
CREATE INDEX idx_heartbeat_monitors_status ON heartbeat_monitors(status);
