use crate::core::db::{async_db, initialize_db};
use crate::core::git::maybe_clone_repo;
use anyhow::{Result, anyhow};
use std::fs;

pub async fn run(
    db: bool,
    index: bool,
    notes: bool,
    vec_db_path: &str,
    index_path: &str,
    notes_path: &str,
) -> Result<()> {
    if !db && !index && !notes {
        return Err(anyhow!(
            "Missing value for init \"--db\", \"--index\", and/or \"--notes\""
        ));
    }

    if db {
        println!("Initializing db...");
        // Initialize the vector DB
        fs::create_dir_all(&vec_db_path)
            .unwrap_or_else(|err| println!("Ignoring vector DB create failed: {}", err));

        let db = async_db(&vec_db_path)
            .await
            .expect("Failed to connect to db");
        db.call(|conn| {
            initialize_db(conn).expect("DB initialization failed");
            Ok(())
        })
        .await?;
        println!("Finished initializing db");
    }

    if index {
        println!("Initializing search index...");
        // Create the index directory if it doesn't already exist
        fs::create_dir_all(&index_path)
            .unwrap_or_else(|err| println!("Ignoring index directory create failed: {}", err));
        println!("Finished initializing search index...");
    }

    // Clone and reset the notes repo to origin/main
    if notes {
        let deploy_key_path =
            std::env::var("HQ_NOTES_DEPLOY_KEY_PATH").expect("Missing env var HQ_NOTES_REPO_URL");
        let repo_url =
            std::env::var("HQ_NOTES_REPO_URL").expect("Missing env var HQ_NOTES_REPO_URL");
        println!("Cloning notes repo from git...");
        maybe_clone_repo(&deploy_key_path, &repo_url, &notes_path).await;
        println!("Finished cloning and resetting notes from git");
    }

    Ok(())
}
