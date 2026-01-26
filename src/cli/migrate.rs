use crate::core::db::{async_db, migrate_db};
use crate::search::fts::utils::recreate_index;
use anyhow::Result;

pub async fn run(db: bool, index: bool, vec_db_path: &str, index_path: &str) -> Result<()> {
    // Run the DB migration script
    if db {
        println!("Migrating db...");
        let db = async_db(&vec_db_path)
            .await
            .expect("Failed to connect to db");
        db.call(|conn| {
            migrate_db(conn).unwrap_or_else(|err| eprintln!("DB migration failed {}", err));
            Ok(())
        })
        .await?;
        println!("Finished migrating db");
    }

    // Delete and recreate the index
    if index {
        println!("Migrating search index...");
        recreate_index(&index_path);
        println!("Finished migrating search index");
        println!("NOTE: You will need to re-populate the index by running --index --full-text");
    }

    Ok(())
}
