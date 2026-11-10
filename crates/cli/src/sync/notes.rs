use super::SyncConfig;
use anyhow::{Result, bail};
use hq::core::db::async_db;
use hq::core::git;
use hq::search::reindex_changed_notes;

/// Sync the notes repo with git and reindex the notes that changed.
///
/// Mirrors the server's periodic `GitSync` job: commit local edits, fetch +
/// rebase on origin, push, then reindex only the files touched by the rebase.
/// Reindexing here (rather than leaving it to the job) matters because the job
/// compares HEAD before and after its own sync — changes pulled by this
/// command would otherwise never be indexed.
pub async fn run(config: &SyncConfig<'_>) -> Result<()> {
    let SyncConfig {
        deploy_key_path,
        notes_path,
        index_path,
        vec_db_path,
    } = *config;

    // Only operate on a directory that is its own git repo. Without this,
    // git commands walk up to an enclosing repo (e.g. the hq checkout) and
    // commit/push that instead.
    if !git::is_git_repo(notes_path) {
        bail!("Notes path is not a git repo: {notes_path}");
    }

    let changed = git::sync_repo(deploy_key_path, notes_path).await?;
    if changed.is_empty() {
        println!("Notes repo already up to date");
        return Ok(());
    }

    let db = async_db(vec_db_path).await?;
    reindex_changed_notes(&db, index_path, notes_path, &changed, true, true).await?;

    println!(
        "Synced notes repo: {} changed file(s) reindexed",
        changed.len()
    );
    Ok(())
}
