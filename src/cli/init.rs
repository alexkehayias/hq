use crate::core::db::{async_db, initialize_db};
use crate::core::git::maybe_clone_repo;
use anyhow::Result;
use tokio::fs;

pub async fn run(
    db: bool,
    index: bool,
    notes: bool,
    skills: bool,
    workspace: bool,
    vec_db_path: &str,
    index_path: &str,
    notes_path: &str,
) -> Result<()> {
    let storage_path = std::env::var("HQ_STORAGE_PATH").unwrap_or_else(|_| "./".to_string());

    if db {
        println!("Initializing db...");
        fs::create_dir_all(vec_db_path).await
            .unwrap_or_else(|err| println!("Ignoring vector DB create failed: {}", err));

        let connection = async_db(vec_db_path)
            .await
            .expect("Failed to connect to db");
        connection.call(|conn| {
            initialize_db(conn).expect("DB initialization failed");
            Ok(())
        }).await?;
        println!("Finished initializing db");
    }

    if index {
        println!("Initializing search index...");
        fs::create_dir_all(index_path).await
            .unwrap_or_else(|err| println!("Ignoring index directory create failed: {}", err));
        println!("Finished initializing search index...");
    }

    if skills {
        let skills_path = format!("{}/skills", storage_path);
        println!("Creating skills directory...");
        fs::create_dir_all(&skills_path).await
            .unwrap_or_else(|err| println!("Ignoring skills directory create failed: {}", err));
        println!("Finished creating skills directory");
    }

    if workspace {
        let workspace_path = format!("{}/workspace", storage_path);
        println!("Creating workspace directory...");
        fs::create_dir_all(&workspace_path).await
            .unwrap_or_else(|err| println!("Ignoring workspace directory create failed: {}", err));
        println!("Finished creating workspace directory");
    }

    // Clone and reset the notes repo to origin/main
    if notes {
        // Always create the notes directory, even if cloning is skipped
        fs::create_dir_all(notes_path).await
            .unwrap_or_else(|err| println!("Ignoring notes directory create failed: {}", err));

        let deploy_key_path =
            std::env::var("HQ_NOTES_DEPLOY_KEY_PATH").unwrap_or_else(|_| "stub".to_string());
        let repo_url =
            std::env::var("HQ_NOTES_REPO_URL").unwrap_or_else(|_| "stub".to_string());

        if repo_url == "stub" || deploy_key_path == "stub" {
            println!("Skipping notes clone: HQ_NOTES_REPO_URL not configured");
        } else {
            println!("Cloning notes repo from git...");
            maybe_clone_repo(&deploy_key_path, &repo_url, notes_path).await;
            println!("Finished cloning and resetting notes from git");
        }
    }

    Ok(())
}
