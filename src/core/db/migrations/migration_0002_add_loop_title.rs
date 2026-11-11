//! Migration 2: add_loop_title
//!
//! Loops now carry a short, human-readable title so they are easier to tell
//! apart. Existing loops are backfilled with a placeholder by the column
//! default; new loops must supply a title.

use super::MigrationFn;
use anyhow::Result;
use tokio_rusqlite::rusqlite::Connection;

pub const DOWN: Option<MigrationFn> = Some(down);

pub fn up(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE loop ADD COLUMN title TEXT NOT NULL DEFAULT 'Untitled loop';",
    )?;
    Ok(())
}

pub fn down(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE loop DROP COLUMN title;")?;
    Ok(())
}
