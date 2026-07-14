-- Small key-value store for job state that must survive restarts (first user: the digest
-- job's window anchor). Not for domain data.
CREATE TABLE IF NOT EXISTS app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
