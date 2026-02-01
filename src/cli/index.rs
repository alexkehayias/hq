use crate::core::git::maybe_pull_and_reset_repo;
use crate::search::indexing::index_all;
use anyhow::{Result, anyhow};
use std::env;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub async fn run(
    all: bool,
    full_text: bool,
    vector: bool,
    index_path: &str,
    notes_path: &str,
    vec_db_path: &str,
) -> Result<()> {
    if !all && !full_text && !vector {
        return Err(anyhow!(
            "Missing value for index \"all\", \"full-text\", and/or \"vector\""
        ));
    }
    // If using the CLI only and not the webserver, set up tracing to
    // output to stdout and stderr
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=debug", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Clone the notes repo
    let deploy_key_path =
        env::var("HQ_NOTES_DEPLOY_KEY_PATH").expect("Missing env var HQ_NOTES_REPO_URL");
    maybe_pull_and_reset_repo(&deploy_key_path, &notes_path).await;

    let db = crate::core::db::async_db(&vec_db_path)
        .await
        .expect("Failed to connect to async db");

    if full_text {
        index_all(&db, &index_path, &notes_path, true, false, None)
            .await
            .expect("Indexing failed");
    }
    if vector {
        index_all(&db, &index_path, &notes_path, false, true, None)
            .await
            .expect("Indexing failed");
    }
    if all {
        index_all(&db, &index_path, &notes_path, true, true, None)
            .await
            .expect("Indexing failed");
    }

    Ok(())
}
