//! Helpers shared by the chat routes that address a session.

use std::path::PathBuf;

use anyhow::Result;
use tokio_rusqlite::Connection;

/// Session IDs are used as a path segment (`workspace/{session_id}`), so
/// restrict them to a safe character set to prevent path traversal.
pub fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether a chat session with this id exists in the database.
pub async fn session_exists(db: &Connection, session_id: &str) -> Result<bool> {
    let session_id = session_id.to_string();
    let exists = db
        .call(move |conn| {
            let mut stmt = conn.prepare("SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)")?;
            let exists: bool = stmt.query_row([session_id], |row| row.get(0))?;
            Ok(exists)
        })
        .await?;
    Ok(exists)
}

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

    let root = workspace_root(storage_path, session_id);
    tokio::fs::create_dir_all(&root).await?;
    Ok(())
}
