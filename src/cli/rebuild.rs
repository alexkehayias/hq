use crate::search::recreate_index;
use crate::search::index_all;
use anyhow::Result;
use std::env;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub async fn run(index_path: &str, notes_path: &str, vec_db_path: &str) -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db = crate::core::db::async_db(&vec_db_path)
        .await
        .expect("Failed to connect to async db");

    // Delete all note metadata and vector data
    println!("Deleting all meta data in the db...");
    db.call(|conn| {
        conn.execute("DELETE FROM vec_items", [])?;
        conn.execute("DELETE FROM note_meta", [])?;
        Ok(())
    })
    .await
    .expect("Failed to delete note_meta or vec_items data");
    println!("Finished deleting all meta data the db...");

    // Remove the full text search index
    println!("Recreating search index...");
    recreate_index(&index_path);
    println!("Finished recreating search index");

    // Index everything
    index_all(&db, &index_path, &notes_path, true, true, None)
        .await
        .expect("Indexing failed");

    Ok(())
}
