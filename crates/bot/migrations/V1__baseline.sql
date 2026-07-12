-- Bot state baseline (ADR 0011). `IF NOT EXISTS` so this is a safe baseline over
-- databases that predate refinery (created by the old CREATE-IF-NOT-EXISTS +
-- ALTER approach); on those it no-ops and refinery just records it as applied.
-- Fresh databases get the full schema here. Later schema changes go in V2+.

CREATE TABLE IF NOT EXISTS deliveries (
    id TEXT PRIMARY KEY,
    received_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL DEFAULT (datetime('now')),
    kind TEXT NOT NULL,
    org TEXT,
    detail TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS releases (
    org TEXT NOT NULL,
    app TEXT NOT NULL,
    version TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    app_image TEXT NOT NULL,
    published_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (org, app, version)
);

CREATE TABLE IF NOT EXISTS imports (
    org TEXT NOT NULL,
    app TEXT NOT NULL,
    status TEXT NOT NULL,
    step TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '',
    request TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (org, app)
);
