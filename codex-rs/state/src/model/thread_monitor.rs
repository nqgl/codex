use std::path::PathBuf;

use anyhow::Result;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// One command monitor associated with a resumable thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMonitor {
    pub name: String,
    pub command: String,
    pub cwd: PathBuf,
    pub trusted: bool,
    pub running: bool,
}

impl ThreadMonitor {
    pub(crate) fn try_from_row(row: &SqliteRow) -> Result<Self> {
        Ok(Self {
            name: row.try_get("name")?,
            command: row.try_get("command")?,
            cwd: PathBuf::from(row.try_get::<String, _>("cwd")?),
            trusted: row.try_get("trusted")?,
            running: row.try_get("running")?,
        })
    }
}
