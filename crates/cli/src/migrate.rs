use anyhow::{Context, Result, bail};
use clap::Subcommand;
use hq::core::db::async_db;
use hq::core::db::migrations;
use hq::core::db::migrations::Status;
use hq::search::recreate_index;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum MigrateCommand {
    /// Apply all pending database migrations (the default)
    Up,
    /// Roll back the most recently applied migrations
    Down {
        /// Number of migrations to roll back
        #[arg(long, default_value_t = 1)]
        steps: usize,
    },
    /// Show applied and pending migrations
    Status,
    /// Check that the migration registry is well-formed
    Validate,
    /// Create a new migration module
    New {
        /// Migration name in snake_case (e.g. add_user_table)
        name: String,
    },
}

pub async fn run(
    command: Option<MigrateCommand>,
    index: bool,
    vec_db_path: &str,
    index_path: &str,
) -> Result<()> {
    let command = command.unwrap_or(MigrateCommand::Up);

    // Checking the registry does not need a database connection.
    if let MigrateCommand::Validate = &command {
        migrations::validate()?;
        println!("Migrations are valid.");
        return Ok(());
    }

    // Generating a file does not need a database connection.
    if let MigrateCommand::New { name } = &command {
        let (path, _, _) = create_migration(name)?;
        println!("Created {}", path.display());
        println!("Implement `up` and `down` in that file, then run `hq migrate`.");
        return Ok(());
    }

    let db = async_db(vec_db_path).await?;

    match command {
        MigrateCommand::Up => {
            let applied = db
                .call(|conn| migrations::migrate(conn).map_err(migrations::db_error))
                .await?;
            if applied.is_empty() {
                println!("No pending migrations.");
            } else {
                for migration in applied {
                    println!(
                        "Applied migration {} ({})",
                        migration.version, migration.name
                    );
                }
            }
        }
        MigrateCommand::Down { steps } => {
            let rolled_back = db
                .call(move |conn| migrations::rollback(conn, steps).map_err(migrations::db_error))
                .await?;
            for migration in rolled_back {
                println!(
                    "Rolled back migration {} ({})",
                    migration.version, migration.name
                );
            }
        }
        MigrateCommand::Status => {
            let status = db
                .call(|conn| migrations::status(conn).map_err(migrations::db_error))
                .await?;
            print_status(&status);
        }
        MigrateCommand::Validate | MigrateCommand::New { .. } => {
            unreachable!("handled before opening the database")
        }
    }

    // Rebuilding the search index is destructive and unrelated to the schema,
    // but it used to live on this command, so keep it available as a flag.
    if index {
        println!("Migrating search index...");
        recreate_index(index_path).await;
        println!("Finished migrating search index");
        println!(
            "NOTE: You will need to re-populate the index by running `index --full-text` and `index --chat`"
        );
    }

    Ok(())
}

fn print_status(status: &Status) {
    if status.applied.is_empty() {
        println!("Applied migrations: none");
    } else {
        println!("Applied migrations:");
        for migration in &status.applied {
            println!(
                "  {:<4} {:<24} {}",
                migration.version, migration.name, migration.applied_at
            );
        }
    }

    if status.pending.is_empty() {
        println!("Pending migrations: none");
    } else {
        println!("Pending migrations:");
        for migration in &status.pending {
            println!("  {:<4} {}", migration.version, migration.name);
        }
    }
}

/// Writes a new migration module and returns its path, module name, and version.
fn create_migration(name: &str) -> Result<(PathBuf, String, i64)> {
    create_migration_in(&migrations_dir()?, name)
}

fn create_migration_in(dir: &Path, name: &str) -> Result<(PathBuf, String, i64)> {
    validate_name(name)?;

    let version = next_version(dir)?;
    let module = format!("migration_{version:04}_{name}");
    let path = dir.join(format!("{module}.rs"));

    if path.exists() {
        bail!("{} already exists", path.display());
    }

    let contents = format!(
        r#"//! Migration {version}: {name}
//!
//! `build.rs` discovers this file automatically; there is no registry to edit.
//! Implement `up` and `down`, then run `hq migrate`.

use super::MigrationFn;
use anyhow::{{Result, bail}};
use tokio_rusqlite::rusqlite::Connection;

pub const DOWN: Option<MigrationFn> = Some(down);

pub fn up(_conn: &Connection) -> Result<()> {{
    // TODO: apply the schema change, for example:
    // _conn.execute_batch("ALTER TABLE note_meta ADD COLUMN foo TEXT;")?;
    bail!("migration {version} ({name}) is not implemented yet")
}}

pub fn down(_conn: &Connection) -> Result<()> {{
    // TODO: reverse the schema change, or set `DOWN` to `None` if it cannot be undone.
    bail!("migration {version} ({name}) is not implemented yet")
}}
"#
    );

    fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))?;

    // Keep the HEAD pointer in sync. Because every new migration touches this
    // same file, two branches that add migrations conflict on merge instead of
    // silently diverging.
    let head_path = dir.join("HEAD");
    fs::write(&head_path, format!("{module}\n"))
        .with_context(|| format!("failed to write {}", head_path.display()))?;

    Ok((path, module, version))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("migration name cannot be empty");
    }
    if !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        bail!("migration name must start with a lowercase letter");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        bail!("migration name must be snake_case (lowercase letters, digits, underscores)");
    }
    Ok(())
}

/// Finds `src/core/db/migrations` by walking up from the current directory.
fn migrations_dir() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("failed to determine current directory")?;
    loop {
        let candidate = dir.join("src/core/db/migrations");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        if !dir.pop() {
            bail!("could not find src/core/db/migrations; run this from the hq repository");
        }
    }
}

/// Picks the next version, considering both the registry and files on disk.
fn next_version(dir: &Path) -> Result<i64> {
    let mut max = migrations::all()
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0);

    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let file_name = entry?.file_name();
        let file_name = file_name.to_string_lossy();
        let Some(rest) = file_name.strip_prefix("migration_") else {
            continue;
        };
        let Some((version, _)) = rest.split_once('_') else {
            continue;
        };
        if let Ok(version) = version.parse::<i64>() {
            max = max.max(version);
        }
    }

    Ok(max + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_migration_writes_next_version() {
        let dir = tempfile::tempdir().unwrap();
        let (path, module, version) = create_migration_in(dir.path(), "add_user_table").unwrap();

        assert_eq!(version, 2);
        assert_eq!(module, "migration_0002_add_user_table");
        assert_eq!(path, dir.path().join("migration_0002_add_user_table.rs"));

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("pub fn up"));
        assert!(contents.contains("pub fn down"));
        assert!(contents.contains("pub const DOWN"));
        assert!(contents.contains("migration 2 (add_user_table)"));

        let head = fs::read_to_string(dir.path().join("HEAD")).unwrap();
        assert_eq!(head, "migration_0002_add_user_table\n");
    }

    #[test]
    fn next_version_considers_files_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("migration_0007_foo.rs"), "").unwrap();
        assert_eq!(next_version(dir.path()).unwrap(), 8);
    }

    #[test]
    fn rejects_invalid_names() {
        let dir = tempfile::tempdir().unwrap();
        assert!(create_migration_in(dir.path(), "").is_err());
        assert!(create_migration_in(dir.path(), "Bad").is_err());
        assert!(create_migration_in(dir.path(), "has-dash").is_err());
        assert!(create_migration_in(dir.path(), "1starts_with_digit").is_err());
    }
}
