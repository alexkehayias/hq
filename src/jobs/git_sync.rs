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
///   1. Non-destructively pulls (rebase local commits on top of origin)
///   2. Commits and pushes any local changes (note edits from API, CLI,
///      or external editors)
///   3. Reindexes files that changed as a result of the rebase (origin's
///      new contributions + our own rebased commit)
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
        // Only sync if notes_path is its own git repo. When running from a dir
        // whose parent is a git repo (e.g. dev in the repo root with no notes
        // clone), git commands would otherwise walk up and operate on that
        // parent repo — the hq repo itself — committing and pushing it.
        if !git::is_git_repo(&config.notes_path) {
            tracing::info!(
                "GitSync: notes path is not a git repo ({}), skipping sync",
                config.notes_path
            );
            return;
        }

        // 1. Pull latest non-destructively (rebase local commits on top of origin)
        if let Err(e) = git::maybe_pull_rebase(&config.deploy_key_path, &config.notes_path).await {
            tracing::error!("GitSync: pull/rebase failed: {e}");
        }

        // 2. Commit and push any local changes; get back files changed by rebase
        match git::sync_repo(&config.deploy_key_path, &config.notes_path).await {
            Ok(changed) if !changed.is_empty() => {
                // 3. Reindex only files that changed as a result of the rebase
                let paths: Vec<PathBuf> = changed
                    .iter()
                    .map(|f| PathBuf::from(format!("{}/{}", &config.notes_path, f)))
                    .collect();
                if let Err(e) = index_all(
                    db_conn,
                    &config.index_path,
                    &config.notes_path,
                    true, // full text
                    true, // vector
                    Some(paths),
                )
                .await
                {
                    tracing::error!("GitSync: reindexing changed files failed: {e}");
                }
            }
            Ok(_) => {} // no changes to reindex
            Err(e) => tracing::error!("GitSync: sync_repo failed: {e}"),
        }
    }
}