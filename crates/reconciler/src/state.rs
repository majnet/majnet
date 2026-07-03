//! Event log — every action tagged with its causing commit (§12 principles).
//! The reconciler carries no state git doesn't; this is an audit trail, not
//! a source of truth.

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, serde::Serialize)]
pub struct Event {
    pub at: String,
    pub commit: String,
    pub project: String,
    pub node: String,
    pub action: String,
    pub result: String,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let conn = Connection::open(dir.join("reconciler.sqlite"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                 seq INTEGER PRIMARY KEY AUTOINCREMENT,
                 at TEXT NOT NULL DEFAULT (datetime('now')),
                 commit_sha TEXT NOT NULL,
                 project TEXT NOT NULL,
                 node TEXT NOT NULL,
                 action TEXT NOT NULL,
                 result TEXT NOT NULL
             );",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn record(&self, commit: &str, project: &str, node: &str, action: &str, result: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (commit_sha, project, node, action, result) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![commit, project, node, action, result],
        )?;
        Ok(())
    }

    pub fn recent(&self, limit: u32) -> Result<Vec<Event>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT at, commit_sha, project, node, action, result FROM events ORDER BY seq DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |row| {
            Ok(Event {
                at: row.get(0)?,
                commit: row.get(1)?,
                project: row.get(2)?,
                node: row.get(3)?,
                action: row.get(4)?,
                result: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}
