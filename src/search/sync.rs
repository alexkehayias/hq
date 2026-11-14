use tokio_rusqlite::Connection;

use super::SYNC_LOCK;
use super::index::reindex_changed_notes;
use crate::core::AppConfig;
use crate::core::git;

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
