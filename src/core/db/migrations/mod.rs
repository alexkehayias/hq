//! A small SQLite migration framework.
//!
//! A migration is a Rust module that exposes an `up` function (and an optional
//! `down` function) operating on a [`rusqlite::Connection`]. The set of
//! migrations applied to a database is recorded in the `schema_migrations`
//! table, so [`migrate`] runs only the pending ones and [`rollback`] runs the
//! most recent ones in reverse order.
//!
//! Each migration runs inside its own transaction and is recorded in the same
//! transaction, so a failure leaves the database at the previous migration.

use anyhow::{Context, Result, bail};
use rusqlite::Connection;
use std::collections::HashSet;

pub mod migration_0001_initial_schema;

/// A single schema migration.
#[derive(Debug, Clone)]
pub struct Migration {
    /// Monotonically increasing version. This is the primary key in
    /// `schema_migrations`.
    pub version: i64,
    /// Human-readable name, stored alongside the version.
    pub name: &'static str,
    /// Applies the migration.
    pub up: fn(&Connection) -> Result<()>,
    /// Reverses the migration. `None` for migrations that cannot be undone.
    pub down: Option<fn(&Connection) -> Result<()>>,
}

/// A migration recorded as applied to a database.
#[derive(Debug, Clone)]
pub struct AppliedMigration {
    pub version: i64,
    pub name: String,
    pub applied_at: String,
}

/// The applied and pending migrations for a database.
#[derive(Debug)]
pub struct Status {
    pub applied: Vec<AppliedMigration>,
    pub pending: Vec<Migration>,
}

/// Every migration known to this build, in version order.
pub fn all() -> Vec<Migration> {
    vec![Migration {
        version: 1,
        name: "initial_schema",
        up: migration_0001_initial_schema::up,
        down: Some(migration_0001_initial_schema::down),
    }]
}

/// Converts a migration error into the error type used by `tokio_rusqlite`.
///
/// Migration functions use `anyhow` for readable context; callers that run
/// them inside [`tokio_rusqlite::Connection::call`] need this conversion.
pub fn db_error(err: anyhow::Error) -> tokio_rusqlite::Error {
    tokio_rusqlite::Error::Other(err.into())
}

const MIGRATIONS_TABLE: &str = "schema_migrations";

fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {MIGRATIONS_TABLE} (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );"
    ))?;
    Ok(())
}

/// Returns the migrations recorded as applied, oldest first.
fn applied(conn: &Connection) -> Result<Vec<AppliedMigration>> {
    ensure_table(conn)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT version, name, applied_at FROM {MIGRATIONS_TABLE} ORDER BY version"
    ))?;
    let rows = stmt.query_map([], |row| {
        Ok(AppliedMigration {
            version: row.get(0)?,
            name: row.get(1)?,
            applied_at: row.get(2)?,
        })
    })?;

    let mut migrations = Vec::new();
    for row in rows {
        migrations.push(row?);
    }
    Ok(migrations)
}

/// Applies every pending migration, in version order.
///
/// Returns the migrations that ran. If there are none, this is a no-op.
pub fn migrate(conn: &Connection) -> Result<Vec<Migration>> {
    let applied = applied(conn)?;
    let applied_versions: HashSet<i64> = applied.iter().map(|m| m.version).collect();
    let known_versions: HashSet<i64> = all().iter().map(|m| m.version).collect();

    // Refuse to run against a database that has seen a migration this build
    // does not know about. This usually means the binary is older than the
    // database, and continuing could corrupt the schema.
    if let Some(unknown) = applied
        .iter()
        .find(|m| !known_versions.contains(&m.version))
    {
        bail!(
            "database contains migration {} ({}) that this build does not know about",
            unknown.version,
            unknown.name
        );
    }

    let pending: Vec<Migration> = all()
        .into_iter()
        .filter(|m| !applied_versions.contains(&m.version))
        .collect();

    // A migration inserted below an already-applied one would be skipped
    // forever. Fail loudly instead of leaving a silently incomplete schema.
    let latest = applied.iter().map(|m| m.version).max().unwrap_or(0);
    if let Some(out_of_order) = pending.iter().find(|m| m.version < latest) {
        bail!(
            "migration {} ({}) is older than the latest applied migration ({})",
            out_of_order.version,
            out_of_order.name,
            latest
        );
    }

    if pending.is_empty() {
        return Ok(pending);
    }

    with_foreign_keys_disabled(conn, || {
        for migration in &pending {
            let tx = conn.unchecked_transaction()?;
            (migration.up)(&tx).with_context(|| {
                format!(
                    "migration {} ({}) failed",
                    migration.version, migration.name
                )
            })?;
            tx.execute(
                &format!("INSERT INTO {MIGRATIONS_TABLE} (version, name) VALUES (?1, ?2)"),
                rusqlite::params![migration.version, migration.name],
            )?;
            tx.commit()?;
        }
        Ok(())
    })?;

    Ok(pending)
}

/// Rolls back the most recent `steps` migrations, newest first.
///
/// Every target is checked for reversibility before anything runs. Each
/// migration then rolls back in its own transaction, so a failure part-way
/// leaves the migrations that already rolled back recorded as such.
pub fn rollback(conn: &Connection, steps: usize) -> Result<Vec<Migration>> {
    let applied = applied(conn)?;
    let registry = all();

    let targets: Vec<AppliedMigration> = applied.into_iter().rev().take(steps).collect();
    if targets.is_empty() {
        bail!("no applied migrations to roll back");
    }

    let mut migrations = Vec::with_capacity(targets.len());
    for target in &targets {
        let migration = registry
            .iter()
            .find(|m| m.version == target.version)
            .with_context(|| {
                format!(
                    "cannot roll back migration {} ({}): not known to this build",
                    target.version, target.name
                )
            })?
            .clone();
        if migration.down.is_none() {
            bail!(
                "migration {} ({}) is not reversible",
                migration.version,
                migration.name
            );
        }
        migrations.push(migration);
    }

    with_foreign_keys_disabled(conn, || {
        for migration in &migrations {
            let down = migration.down.expect("checked above");
            let tx = conn.unchecked_transaction()?;
            down(&tx).with_context(|| {
                format!(
                    "rollback of migration {} ({}) failed",
                    migration.version, migration.name
                )
            })?;
            tx.execute(
                &format!("DELETE FROM {MIGRATIONS_TABLE} WHERE version = ?1"),
                [migration.version],
            )?;
            tx.commit()?;
        }
        Ok(())
    })?;

    Ok(migrations)
}

/// Reports which migrations are applied and which are still pending.
pub fn status(conn: &Connection) -> Result<Status> {
    let applied = applied(conn)?;
    let applied_versions: HashSet<i64> = applied.iter().map(|m| m.version).collect();
    let pending = all()
        .into_iter()
        .filter(|m| !applied_versions.contains(&m.version))
        .collect();
    Ok(Status { applied, pending })
}

/// Runs `f` with foreign key enforcement disabled.
///
/// Table rebuilds drop tables that other tables reference, and SQLite checks
/// those references at statement time. Migrations are responsible for ending
/// with a consistent schema; the check is restored before returning.
fn with_foreign_keys_disabled<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let enabled: bool = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .map(|value| value != 0)
        .unwrap_or(false);

    if enabled {
        conn.execute_batch("PRAGMA foreign_keys=OFF;")?;
    }

    let result = f();

    if enabled {
        match conn.execute_batch("PRAGMA foreign_keys=ON;") {
            Ok(()) => {}
            Err(err) if result.is_ok() => return Err(err.into()),
            Err(_) => {}
        }
    }

    result
}
