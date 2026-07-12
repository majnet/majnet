-- Reconciler state baseline (ADR 0011). `IF NOT EXISTS` so this is a safe
-- baseline over databases that predate refinery; on those it no-ops and
-- refinery records it as applied. Fresh databases get the full schema here.
-- Later schema changes go in V2+.

CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL DEFAULT (datetime('now')),
    commit_sha TEXT NOT NULL,
    project TEXT NOT NULL,
    node TEXT NOT NULL,
    action TEXT NOT NULL,
    result TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ephemeral_stacks (
    project TEXT NOT NULL,
    app TEXT NOT NULL,
    first_deployed TEXT NOT NULL DEFAULT (datetime('now')),
    missing_since TEXT,
    extended_until TEXT,
    PRIMARY KEY (project, app)
);

CREATE TABLE IF NOT EXISTS data_migrations (
    project TEXT NOT NULL,
    app TEXT NOT NULL,
    class TEXT NOT NULL,
    done_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (project, app, class)
);
