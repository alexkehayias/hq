use sqlite_vec::sqlite3_vec_init;
use std::os::raw::c_char;
use std::time::Duration;
use tokio_rusqlite::{Connection, ffi::sqlite3_auto_extension, rusqlite};

pub mod migrations;

/// Initialize the db by running all pending migrations.
///
/// This is safe to run multiple times: migrations already recorded in
/// `schema_migrations` are skipped. See [`migrations`] for the framework.
pub fn initialize_db(db: &rusqlite::Connection) -> Result<(), migrations::MigrationError> {
    migrations::migrate(db).map_err(migrations::db_error)?;
    Ok(())
}

pub async fn async_db(path_to_db_file: &str) -> anyhow::Result<Connection, anyhow::Error> {
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(
                *mut tokio_rusqlite::ffi::sqlite3,
                *mut *mut c_char,
                *const tokio_rusqlite::ffi::sqlite3_api_routines,
            ) -> i32,
        >(sqlite3_vec_init as *const ())));
    }
    let db = Connection::open(format!("{}/vector.db", path_to_db_file)).await?;

    // Dokku's release step migrates the database while the previous container
    // may still be serving requests. Wait briefly for a write lock instead of
    // failing the deploy immediately with SQLITE_BUSY.
    db.call(|conn| conn.busy_timeout(Duration::from_secs(5)))
        .await?;

    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pre-migration `loop` table has a nullable `system_prompt`. The
    /// baseline migration must tighten it to NOT NULL and drop the loops (and
    /// their channel links) that never had a prompt.
    #[tokio::test]
    async fn migration_makes_loop_system_prompt_required() {
        let dir = tempfile::tempdir().unwrap();
        let db = async_db(dir.path().to_str().unwrap()).await.unwrap();

        db.call(|conn| {
            initialize_db(conn)?;

            // Revert just the loop table to its old nullable form to simulate
            // an upgrade, then forget that the baseline migration ran so it
            // runs again.
            conn.execute_batch(
                "DROP TABLE loop_channel;
                 DROP TABLE loop;
                 CREATE TABLE loop (
                     id TEXT PRIMARY KEY,
                     system_prompt TEXT,
                     tools TEXT NOT NULL,
                     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                 );
                 CREATE TABLE loop_channel (
                     loop_id TEXT NOT NULL REFERENCES loop(id),
                     channel_id TEXT NOT NULL REFERENCES channel(id),
                     position INTEGER NOT NULL,
                     PRIMARY KEY (loop_id, channel_id)
                 );
                 INSERT INTO channel (id, source) VALUES ('alpha', 'socket');
                 INSERT INTO loop (id, system_prompt, tools)
                     VALUES ('kept', 'Do the thing.', '[\"bash\"]');
                 INSERT INTO loop (id, system_prompt, tools)
                     VALUES ('dropped', NULL, '[\"bash\"]');
                 INSERT INTO loop_channel (loop_id, channel_id, position)
                     VALUES ('kept', 'alpha', 0), ('dropped', 'alpha', 0);
                 DELETE FROM schema_migrations;",
            )?;

            initialize_db(conn)?;

            let not_null: bool = conn
                .query_row(
                    "SELECT \"notnull\" = 1 FROM pragma_table_info('loop') WHERE name = 'system_prompt'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(not_null, "system_prompt should be NOT NULL after migrating");

            let ids: Vec<String> = conn
                .prepare("SELECT id FROM loop ORDER BY id")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .filter_map(|row| row.ok())
                .collect();
            assert_eq!(ids, vec!["kept"]);

            // The title migration backfills pre-existing loops with a
            // placeholder so the required column can be added.
            let title: String = conn
                .query_row("SELECT title FROM loop WHERE id = 'kept'", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(title, "Untitled loop");

            let links: i64 = conn
                .query_row("SELECT COUNT(*) FROM loop_channel", [], |row| row.get(0))
                .unwrap();
            assert_eq!(
                links, 1,
                "the dropped loop's channel link should be removed"
            );

            Ok::<_, migrations::MigrationError>(())
        })
        .await
        .unwrap();
    }

    /// `migrate` is idempotent and records what it applied, and `rollback`
    /// undoes the most recent migration.
    #[tokio::test]
    async fn migrate_records_and_rolls_back_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let db = async_db(dir.path().to_str().unwrap()).await.unwrap();

        db.call(|conn| {
            let applied = migrations::migrate(conn).map_err(migrations::db_error)?;
            assert_eq!(applied.len(), migrations::all().len());

            // A second run has nothing to do.
            let again = migrations::migrate(conn).map_err(migrations::db_error)?;
            assert!(again.is_empty());

            let status = migrations::status(conn).map_err(migrations::db_error)?;
            assert!(status.pending.is_empty());
            assert_eq!(status.applied.len(), migrations::all().len());

            let rolled_back = migrations::rollback(conn, 1).map_err(migrations::db_error)?;
            assert_eq!(rolled_back.len(), 1);

            let status = migrations::status(conn).map_err(migrations::db_error)?;
            assert_eq!(status.pending.len(), 1);

            Ok::<_, migrations::MigrationError>(())
        })
        .await
        .unwrap();
    }
}
