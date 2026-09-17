//! Shared notes reindexing: reindex changed note files, optionally syncing the
//! notes repo first. Used by the CLI `index` command, the `GitSync` job, the
//! `/notes/index` endpoint, and the GitHub push webhook.

use std::path::PathBuf;

use anyhow::Result;
use tokio_rusqlite::Connection;

use crate::core::{AppConfig, git};
use crate::search::index_all;

/// Serializes sync+reindex across the periodic `GitSync` job, the `/notes/index`
/// endpoint, and the GitHub webhook. Concurrent runs race on the notes working
/// tree (git rebase/push) and on Tantivy's index writer, which panics when the
/// directory lock is already held.
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Reindex the given note files, whose paths are relative to the notes repo
/// root (as returned by git).
pub async fn reindex_changed_notes(
    db: &Connection,
    index_path: &str,
    notes_path: &str,
    changed: &[String],
    full_text: bool,
    vector: bool,
) -> Result<()> {
    let paths: Vec<PathBuf> = changed
        .iter()
        .map(|f| PathBuf::from(notes_path).join(f))
        .collect();
    index_all(db, index_path, notes_path, full_text, vector, Some(paths)).await
}

/// Commit local note changes, pull origin, and push (via `sync_repo`), then
/// reindex only the files that changed as a result of the rebase.
pub async fn sync_and_reindex_notes(db: &Connection, config: &AppConfig) {
    let _guard = SYNC_LOCK.lock().await;

    // Only sync if notes_path is its own git repo. When running from a dir
    // whose parent is a git repo (e.g. dev in the repo root with no notes
    // clone), git commands would otherwise walk up and operate on that
    // parent repo — the hq repo itself — committing and pushing it.
    if !git::is_git_repo(&config.notes_path) {
        tracing::info!(
            "sync_and_reindex_notes: notes path is not a git repo ({}), skipping sync",
            config.notes_path
        );
        return;
    }

    // sync_repo stages and commits before rebasing, so it (unlike
    // maybe_pull_rebase) tolerates an otherwise-dirty working tree.
    match git::sync_repo(&config.deploy_key_path, &config.notes_path).await {
        Ok(changed) if !changed.is_empty() => {
            if let Err(e) = reindex_changed_notes(
                db,
                &config.index_path,
                &config.notes_path,
                &changed,
                true,
                true,
            )
            .await
            {
                tracing::error!("sync_and_reindex_notes: reindexing changed files failed: {e}");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::error!("sync_and_reindex_notes: sync_repo failed: {e}"),
    }
}
