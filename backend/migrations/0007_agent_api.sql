-- Agent-friendly API pass: token scopes, heartbeat transition history, event full-text search.

-- 'full' = read/write (the historical behavior), 'read' = GET/HEAD only.
ALTER TABLE api_tokens ADD COLUMN scope TEXT NOT NULL DEFAULT 'full';

-- Every status flip of a heartbeat monitor, newest queried first. Rows are pruned by the
-- retention job (same CRASHBOX_RETENTION_DAYS window as events).
CREATE TABLE heartbeat_transitions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  monitor_id INTEGER NOT NULL,
  from_status TEXT NOT NULL,
  to_status TEXT NOT NULL,
  at TEXT NOT NULL,
  FOREIGN KEY (monitor_id) REFERENCES heartbeat_monitors(id) ON DELETE CASCADE
);

CREATE INDEX idx_heartbeat_transitions_monitor ON heartbeat_transitions(monitor_id, at DESC);

-- Full-text index over the verbatim event payload (external-content FTS5 table, so the JSON
-- is not stored twice). raw_json is written once at ingest and never updated, so insert and
-- delete triggers are enough to keep the index in sync — including deletes issued by the
-- retention job.
CREATE VIRTUAL TABLE events_fts USING fts5(raw_json, content='events', content_rowid='id');

INSERT INTO events_fts(rowid, raw_json) SELECT id, raw_json FROM events;

CREATE TRIGGER events_fts_after_insert AFTER INSERT ON events BEGIN
  INSERT INTO events_fts(rowid, raw_json) VALUES (new.id, new.raw_json);
END;

CREATE TRIGGER events_fts_after_delete AFTER DELETE ON events BEGIN
  INSERT INTO events_fts(events_fts, rowid, raw_json) VALUES ('delete', old.id, old.raw_json);
END;
