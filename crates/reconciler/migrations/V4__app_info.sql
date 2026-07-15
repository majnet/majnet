-- Per-app build metadata reported by the app's standard `/info` endpoint.
-- The reconciler scrapes `/info` right after the blue-green health gate passes
-- (when it has already proven the new container serves HTTP) and upserts the
-- result here keyed by (project, app, class). This is a cache of what git +
-- the running container reported at deploy time — not a source of truth — so the
-- dashboard can show each app's self-reported version/commit without probing
-- containers on every page load. `info` is the raw JSON the app returned (NULL
-- when unavailable); `error` records why the probe failed, if it did.
CREATE TABLE IF NOT EXISTS app_info (
    project    TEXT NOT NULL,
    app        TEXT NOT NULL,
    class      TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    info       TEXT,
    error      TEXT,
    at         TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (project, app, class)
);
