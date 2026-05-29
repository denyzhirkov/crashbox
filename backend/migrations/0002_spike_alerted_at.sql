-- Spike detection cooldown: last time we fired a spike alert for this issue.
-- NULL = never alerted; otherwise RFC-3339 UTC timestamp. Filter in spike SQL excludes
-- issues whose last alert was within the cooldown window.
ALTER TABLE issues ADD COLUMN spike_alerted_at TEXT;
