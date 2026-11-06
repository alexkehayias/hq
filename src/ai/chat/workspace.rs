//! The per-session workspace directory shared by the bash sandbox, agent
//! tools, and the chat UI.

use std::path::PathBuf;

use anyhow::Result;

use super::models::is_valid_session_id;

/// Host directory backing a session's workspace, mounted at `/` in the
/// sandbox.
pub fn workspace_root(storage_path: &str, session_id: &str) -> PathBuf {
    PathBuf::from(storage_path)
        .join("workspace")
        .join(session_id)
}

/// Create the session's workspace directory on the host if it doesn't exist.
///
/// Called when a session is created so the terminal and workspace endpoints
/// always have a directory to point at.
pub async fn ensure_workspace(storage_path: &str, session_id: &str) -> Result<()> {
    if !is_valid_session_id(session_id) {
        anyhow::bail!("Invalid session_id: {session_id}");
    }

    tokio::fs::create_dir_all(workspace_root(storage_path, session_id)).await?;
    Ok(())
}
