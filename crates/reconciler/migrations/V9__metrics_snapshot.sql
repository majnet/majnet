-- Latest full fleet metrics snapshot (running state), so the dashboard loads an
-- app's Environment card instantly instead of waiting on a live per-node Docker
-- sweep. The 15s sampler upserts the whole `Vec<NodeMetrics>` (incl. per-
-- container image + state) as one JSON blob; `GET /api/metrics` serves it. Unlike
-- the *_samples history tables this keeps only the current value, not a series.
-- Runtime observability, not platform state — reconciler DB only, never git.
CREATE TABLE IF NOT EXISTS metrics_snapshot (
    id   INTEGER PRIMARY KEY CHECK (id = 0),  -- single row
    ts   INTEGER NOT NULL,                    -- unix seconds the snapshot was taken
    json TEXT    NOT NULL                     -- serialized Vec<NodeMetrics>
);
