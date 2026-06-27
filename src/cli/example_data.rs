use crate::core::db::{async_db, initialize_db};
use crate::search::index_all;
use anyhow::Result;
use std::fs;
use std::path::Path;

/// Copy example .org notes from the repo into the storage notes directory,
/// then index them for search. Safe to run multiple times.
pub async fn run(notes_path: &str, index_path: &str, vec_db_path: &str) -> Result<()> {
    let examples_dir = "./examples/notes";
    let examples_path = Path::new(examples_dir);

    if !examples_path.exists() {
        println!("Example notes directory not found at {}", examples_dir);
        println!("Run this command from the project root directory.");
        return Ok(());
    }

    // Ensure the target notes, db, and index directories exist
    fs::create_dir_all(notes_path)
        .unwrap_or_else(|err| println!("Ignoring notes directory create failed: {}", err));
    fs::create_dir_all(vec_db_path)
        .unwrap_or_else(|err| println!("Ignoring vector DB create failed: {}", err));
    fs::create_dir_all(index_path)
        .unwrap_or_else(|err| println!("Ignoring index directory create failed: {}", err));

    // Copy each .org file from examples to the notes directory
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(examples_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "org") {
                let file_name = path.file_name().unwrap();
                let dest = Path::new(notes_path).join(file_name);
                match fs::copy(&path, &dest) {
                    Ok(_) => {
                        println!("  Copied {}", file_name.to_string_lossy());
                        count += 1;
                    }
                    Err(e) => {
                        println!(
                            "  Warning: failed to copy {}: {}",
                            file_name.to_string_lossy(),
                            e
                        );
                    }
                }
            }
        }
    }

    if count == 0 {
        println!("No .org files found in {}", examples_dir);
        return Ok(());
    }

    println!("Copied {} example notes to {}", count, notes_path);

    // Ensure the database is initialized
    let db = async_db(vec_db_path).await?;
    db.call(|conn| {
        initialize_db(conn)
    })
    .await?;

    // Index the example notes so they're immediately searchable
    println!("Indexing example notes...");
    let cache_dir = std::env::var("HQ_FASTEMBED_CACHE_DIR")
        .unwrap_or_else(|_| ".fastembed_cache".to_string());
    index_all(&db, index_path, notes_path, true, true, None, &cache_dir).await?;
    println!("Finished indexing example notes");

    Ok(())
}
