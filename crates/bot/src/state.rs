//! Persistent state — SQLite. Deliberately minimal: the bot carries no state
//! git doesn't, except webhook delivery dedup and an audit log of actions.

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

/// A release as recorded in the store (ADR 0009), also the dashboard shape. A
/// release is a `vX.Y.Z`-tagged image publish; the migration lives in the ops
/// overlay, not here.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredRelease {
    pub app: String,
    pub version: String,
    pub commit: String,
    pub app_image: String,
    pub published_at: String,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let conn = Connection::open(dir.join("bot.sqlite"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS deliveries (
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
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Returns true if this delivery ID is new (and records it).
    pub fn record_delivery(&self, delivery_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO deliveries (id) VALUES (?1)",
            [delivery_id],
        )?;
        Ok(inserted == 1)
    }

    pub fn log_event(&self, kind: &str, org: Option<&str>, detail: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (kind, org, detail) VALUES (?1, ?2, ?3)",
            rusqlite::params![kind, org, detail],
        )?;
        Ok(())
    }

    /// Record (or update) a release for `org/app` (ADR 0009). Keyed by version,
    /// so a re-published tag overwrites its digest rather than duplicating;
    /// `published_at` keeps its first-seen value (the ordering key).
    pub fn upsert_release(
        &self,
        org: &str,
        app: &str,
        version: &str,
        commit: &str,
        app_image: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO releases (org, app, version, commit_sha, app_image)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(org, app, version) DO UPDATE SET
                 commit_sha = excluded.commit_sha,
                 app_image = excluded.app_image",
            rusqlite::params![org, app, version, commit, app_image],
        )?;
        Ok(())
    }

    /// Releases for `org/app`, newest first.
    pub fn releases(&self, org: &str, app: &str) -> Result<Vec<StoredRelease>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT app, version, commit_sha, app_image, published_at
             FROM releases WHERE org = ?1 AND app = ?2 ORDER BY published_at DESC, version DESC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![org, app], |row| {
                Ok(StoredRelease {
                    app: row.get(0)?,
                    version: row.get(1)?,
                    commit: row.get(2)?,
                    app_image: row.get(3)?,
                    published_at: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}
