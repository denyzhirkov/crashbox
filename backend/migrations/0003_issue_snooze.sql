-- Issue snooze: temporarily mute an issue without resolving it.
-- Value semantics:
--   NULL                 → not snoozed
--   'forever'            → snoozed until the next event lands on this fingerprint
--                          (the ingest path auto-wakes it)
--   '<ISO-8601>'         → snoozed until that UTC instant; lists treat it as snoozed while
--                          snoozed_until > now()
ALTER TABLE issues ADD COLUMN snoozed_until TEXT;
