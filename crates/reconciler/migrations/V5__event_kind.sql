-- Reliable activity type: a `kind` column on the event log so the dashboard's
-- Activity feed classifies/filters events without re-parsing the free-text
-- `action`/`result` (record() sets it authoritatively going forward). Backfill
-- existing rows by the same rule record() uses.
ALTER TABLE events ADD COLUMN kind TEXT;

UPDATE events SET kind = CASE
  WHEN action LIKE 'converge%' OR action LIKE 'deploy%'
       OR action LIKE 'restart%' OR action LIKE 'promote%' THEN 'deploy'
  WHEN action LIKE 'gc%' OR action LIKE 'purge%' OR action LIKE 'remove%' THEN 'remove'
  ELSE 'config'
END;
