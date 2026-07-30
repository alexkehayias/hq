use crate::core::git::{changed_files_between, head_sha, maybe_pull_rebase};
use crate::search::{index_all, index_all_chat_sessions};
use anyhow::{Result, anyhow};
use std::env;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub async fn run(
    all: bool,
    full_text: bool,
    vector: bool,
    chat: bool,
    index_path: &str,
    notes_path: &str,
    vec_db_path: &str,
) -> Result<()> {
    if !all && !full_text && !vector && !chat {
        return Err(anyhow!(
            "Missing value for index \"all\", \"full-text\", \"vector\", and/or \"chat\""
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

    let deploy_key_path =
        env::var("HQ_NOTES_DEPLOY_KEY_PATH").expect("Missing env var HQ_NOTES_REPO_URL");

    // Non-destructive pull: capture HEAD before, rebase local commits on top of origin.
    // This replaces the old destructive `git reset --hard origin/main` which clobbered
    // local changes.
    let pre_head = head_sha(notes_path).await?;
    if let Err(e) = maybe_pull_rebase(&deploy_key_path, notes_path).await {
        tracing::warn!("git pull/rebase failed (continuing with local state): {e}");
    }

    // Compute files that changed as a result of the pull/rebase (origin's new
    // contributions + our own rebased commits). Only these get reindexed — no full
    // reindex. If nothing changed, paths is empty and index_all does nothing.
    let changed = changed_files_between(notes_path, &pre_head, "HEAD")
        .await
        .unwrap_or_default();
    let paths: Vec<PathBuf> = changed
        .iter()
        .map(|f| PathBuf::from(format!("{}/{f}", notes_path)))
        .collect();
    let filter_paths = Some(paths);

    let db = crate::core::db::async_db(vec_db_path)
        .await
        .expect("Failed to connect to async db");

    if full_text {
        index_all(&db, index_path, notes_path, true, false, filter_paths.clone())
            .await
            .expect("Indexing failed");
    }
    if vector {
        index_all(&db, index_path, notes_path, false, true, filter_paths.clone())
            .await
            .expect("Indexing failed");
    }
    if all {
        index_all(&db, index_path, notes_path, true, true, filter_paths.clone())
            .await
            .expect("Indexing failed");
    }
    if chat {
        index_all_chat_sessions(&db, index_path)
            .await
            .expect("Chat indexing failed");
    }

    Ok(())
}