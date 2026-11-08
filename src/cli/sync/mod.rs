use anyhow::Result;
use clap::Subcommand;

pub mod notes;

/// Shared paths and credentials for every sync target.
///
/// Env vars are read once in `run_dispatch` and threaded through here so the
/// subcommand modules stay free of environment access.
pub struct SyncConfig<'a> {
    pub deploy_key_path: &'a str,
    pub notes_path: &'a str,
    pub index_path: &'a str,
    pub vec_db_path: &'a str,
}

#[derive(Subcommand)]
pub enum SyncCommand {
    /// Sync the notes repo with git (commit, pull, push) and reindex changes
    Notes,
}

pub async fn run(command: SyncCommand, config: &SyncConfig<'_>) -> Result<()> {
    match command {
        SyncCommand::Notes => notes::run(config).await,
    }
}
