use async_trait::async_trait;
use std::path::PathBuf;
use std::time::Duration;
use tokio_rusqlite::Connection;

use super::PeriodicJob;
use crate::core::{AppConfig, git};
use crate::search::index_all;

/// Periodic job that syncs the notes repo to git.
///
/// Every 5 minutes:
///   1. Commits any local changes (note edits from API, CLI, or external
///      editors) on top of origin, then pushes (fetch + rebase + push)
///   2. Reindexes files that changed as a result of the rebase (origin's
///      new contributions + our own commit)
///
/// On conflict, `sync_repo` aborts the rebase and logs an error; the next
/// tick retries. On push failure (remote moved), it logs a warning and
/// retries next tick.
#[derive(Debug)]
pub struct GitSync;

#[async_trait]
impl PeriodicJob for GitSync {
    fn interval(&self) -> Duration {
        // Every 5 minutes
        Duration::from_secs(300)
    }

    async fn run_job(&self, config: &AppConfig, db_conn: &Connection) {
        sync_and_reindex_notes(db_conn, config).await;
    }
}

/// Serializes sync+reindex across the periodic `GitSync` job, the `/notes/index`
/// endpoint, and the GitHub webhook. Concurrent runs race on the notes working
/// tree (git rebase/push) and on Tantivy's index writer, which panics when the
/// directory lock is already held.
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Commit local note changes, pull origin, and push (via `sync_repo`), then
/// reindex only the files that changed as a result of the rebase. Shared by the
/// periodic `GitSync` job, the manual `/notes/index` endpoint, and the GitHub
/// push webhook.
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
            let paths: Vec<PathBuf> = changed
                .iter()
                .map(|f| PathBuf::from(&config.notes_path).join(f))
                .collect();
            if let Err(e) = index_all(
                db,
                &config.index_path,
                &config.notes_path,
                true, // full text
                true, // vector
                Some(paths),
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
