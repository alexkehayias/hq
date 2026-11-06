//! Session terminal: run bashkit commands against a session's workspace.
//!
//! Each session gets one long-lived [`Bash`] instance, so working directory,
//! variables, and functions persist across commands. The shell's filesystem is
//! the session workspace (`{storage}/workspace/{session_id}`) mounted at `/`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio::sync::Mutex as AsyncMutex;

use super::public::{TerminalCwdResponse, TerminalRequest, TerminalResponse};
use crate::ai::chat::db::session_exists;
use crate::ai::chat::models::is_valid_session_id;
use crate::ai::chat::workspace::workspace_root;
use crate::ai::tools::new_sandbox;
use crate::api::public::ApiError;
use crate::api::state::AppState;
use crate::bash::Bash;

type SharedState = Arc<RwLock<AppState>>;

/// Maximum number of live shells kept in memory. Beyond this, the least
/// recently used shell is dropped; its next command starts a fresh shell
/// (files in the workspace are unaffected).
const MAX_SHELLS: usize = 32;

struct ShellEntry {
    shell: Arc<AsyncMutex<Bash>>,
    last_used: Instant,
}

/// One shell per session, keyed by session id. The async mutex serializes
/// commands within a session while letting different sessions run concurrently.
#[derive(Clone, Default)]
pub struct TerminalRegistry {
    shells: Arc<Mutex<HashMap<String, ShellEntry>>>,
}

impl TerminalRegistry {
    fn get(&self, session_id: &str) -> Option<Arc<AsyncMutex<Bash>>> {
        let mut shells = self.shells.lock().expect("terminal registry lock poisoned");
        let entry = shells.get_mut(session_id)?;
        entry.last_used = Instant::now();
        Some(Arc::clone(&entry.shell))
    }

    /// Insert a newly built shell, returning the one that ended up in the
    /// registry (an existing shell wins a race with a concurrent first request).
    fn insert(&self, session_id: &str, shell: Arc<AsyncMutex<Bash>>) -> Arc<AsyncMutex<Bash>> {
        let mut shells = self.shells.lock().expect("terminal registry lock poisoned");

        if let Some(existing) = shells.get(session_id) {
            return Arc::clone(&existing.shell);
        }

        if shells.len() >= MAX_SHELLS
            && let Some(oldest) = shells
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| id.clone())
        {
            shells.remove(&oldest);
        }

        shells.insert(
            session_id.to_string(),
            ShellEntry {
                shell: Arc::clone(&shell),
                last_used: Instant::now(),
            },
        );
        shell
    }
}

/// Look up a session's shell, creating it on first use. Concurrent first
/// requests may build two shells; the loser is dropped.
async fn get_or_create_shell(
    registry: &TerminalRegistry,
    storage_path: &str,
    workspace_path: &Path,
    session_id: &str,
) -> Result<Arc<AsyncMutex<Bash>>, ApiError> {
    if let Some(shell) = registry.get(session_id) {
        return Ok(shell);
    }

    let bash = new_sandbox(storage_path, workspace_path, "/").await?;
    Ok(registry.insert(session_id, Arc::new(AsyncMutex::new(bash))))
}

/// Return the shell's current working directory without running a command.
///
/// Used by the page to restore the prompt after a reload. A session without a
/// live shell reports the root, which is where a new shell starts.
pub async fn terminal_cwd_handler(
    State(state): State<SharedState>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    if !is_valid_session_id(&session_id) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
    }

    let (db, registry) = {
        let state = state.read().expect("Unable to read shared state");
        (state.db.clone(), state.terminals.clone())
    };

    if !session_exists(&db, &session_id).await? {
        return Ok((StatusCode::NOT_FOUND, "Session not found").into_response());
    }

    let cwd = match registry.get(&session_id) {
        Some(shell) => {
            let bash = shell.lock().await;
            bash.cwd().to_string_lossy().to_string()
        }
        None => "/".to_string(),
    };

    Ok(Json(TerminalCwdResponse { cwd }).into_response())
}

/// Run a command in the session's persistent bashkit shell.
pub async fn terminal_handler(
    State(state): State<SharedState>,
    AxumPath(session_id): AxumPath<String>,
    Json(body): Json<TerminalRequest>,
) -> Result<Response, ApiError> {
    if !is_valid_session_id(&session_id) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
    }

    let (storage_path, db, registry) = {
        let state = state.read().expect("Unable to read shared state");
        (
            state.config.storage_path.clone(),
            state.db.clone(),
            state.terminals.clone(),
        )
    };

    if !session_exists(&db, &session_id).await? {
        return Ok((StatusCode::NOT_FOUND, "Session not found").into_response());
    }

    let workspace_path = workspace_root(&storage_path, &session_id);
    let workspace_exists = tokio::fs::metadata(&workspace_path)
        .await
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false);
    if !workspace_exists {
        return Ok((StatusCode::NOT_FOUND, "Workspace not found").into_response());
    }

    let shell = get_or_create_shell(&registry, &storage_path, &workspace_path, &session_id).await?;

    let mut bash = shell.lock().await;
    let result = bash.exec(&body.command).await;

    // Command-level failures (bad syntax, unsupported constructs, resource
    // limits) are reported as shell errors with a non-zero exit code so the
    // terminal can render them like any other command output.
    let response = match result {
        Ok(output) => TerminalResponse {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.exit_code,
            truncated: output.stdout_truncated || output.stderr_truncated,
            cwd: bash.cwd().to_string_lossy().to_string(),
        },
        Err(e) => TerminalResponse {
            stdout: String::new(),
            stderr: format!("bash: {e}"),
            exit_code: 1,
            truncated: false,
            cwd: bash.cwd().to_string_lossy().to_string(),
        },
    };

    Ok(Json(response).into_response())
}
