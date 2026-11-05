//! Session workspace files: list and preview what a session owns.
//!
//! The terminal and the agent's bash tool both operate on
//! `{storage}/workspace/{session_id}`. These handlers expose that directory to
//! the chat UI's workspace panel without letting a request escape it.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio::io::AsyncReadExt;

use super::public::{WorkspaceFileEntry, WorkspaceFileResponse, WorkspaceFilesResponse};
use super::session::{is_valid_session_id, session_exists, workspace_root};
use crate::api::public::ApiError;
use crate::api::state::AppState;

type SharedState = Arc<RwLock<AppState>>;

/// Maximum number of entries returned for one listing.
const MAX_ENTRIES: usize = 2_000;

/// Maximum directory depth walked when listing.
const MAX_DEPTH: usize = 8;

/// Maximum bytes returned when previewing a file.
const MAX_PREVIEW_BYTES: usize = 256 * 1024;

/// Reduce a client-supplied path to a relative path with no `..`, root, or
/// prefix components. Returns `None` for anything that could escape the
/// workspace.
fn sanitize_relative_path(raw: &str) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if clean.as_os_str().is_empty() {
        return None;
    }
    Some(clean)
}

/// Recursively collect files and directories. Hidden entries and symlinks are
/// skipped so a listing can't wander outside the workspace or expose internal
/// metadata.
fn walk_workspace(base: &Path, dir: &Path, depth: usize, files: &mut Vec<WorkspaceFileEntry>) {
    if depth > MAX_DEPTH || files.len() >= MAX_ENTRIES {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if files.len() >= MAX_ENTRIES {
            return;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }

        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }

        let path = entry.path();
        let relative = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        if file_type.is_dir() {
            files.push(WorkspaceFileEntry {
                path: relative,
                is_directory: true,
            });
            walk_workspace(base, &path, depth + 1, files);
        } else if file_type.is_file() {
            files.push(WorkspaceFileEntry {
                path: relative,
                is_directory: false,
            });
        }
    }
}

/// List every file and directory in the session workspace.
pub async fn list_workspace_files(
    State(state): State<SharedState>,
    AxumPath(session_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    if !is_valid_session_id(&session_id) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
    }

    let (db, storage_path) = {
        let state = state.read().expect("Unable to read shared state");
        (state.db.clone(), state.config.storage_path.clone())
    };

    if !session_exists(&db, &session_id).await? {
        return Ok((StatusCode::NOT_FOUND, "Session not found").into_response());
    }

    let root = workspace_root(&storage_path, &session_id);

    let files = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        walk_workspace(&root, &root, 0, &mut files);
        files
    })
    .await?;

    Ok(Json(WorkspaceFilesResponse { files }).into_response())
}

/// Return a text preview of a file in the session workspace.
pub async fn read_workspace_file(
    State(state): State<SharedState>,
    AxumPath((session_id, path)): AxumPath<(String, String)>,
) -> Result<Response, ApiError> {
    if !is_valid_session_id(&session_id) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
    }

    let Some(relative) = sanitize_relative_path(&path) else {
        return Ok((StatusCode::BAD_REQUEST, "Invalid path").into_response());
    };

    let (db, storage_path) = {
        let state = state.read().expect("Unable to read shared state");
        (state.db.clone(), state.config.storage_path.clone())
    };

    if !session_exists(&db, &session_id).await? {
        return Ok((StatusCode::NOT_FOUND, "Session not found").into_response());
    }

    let root = workspace_root(&storage_path, &session_id);
    let target = root.join(&relative);

    // Resolve symlinks before reading so a link inside the workspace cannot
    // point at a file outside it.
    let canonical_root = match tokio::fs::canonicalize(&root).await {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((StatusCode::NOT_FOUND, "File not found").into_response());
        }
        Err(e) => return Err(e.into()),
    };
    let canonical_target = match tokio::fs::canonicalize(&target).await {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((StatusCode::NOT_FOUND, "File not found").into_response());
        }
        Err(e) => return Err(e.into()),
    };

    if !canonical_target.starts_with(&canonical_root) {
        return Ok((StatusCode::FORBIDDEN, "Path traversal detected").into_response());
    }

    let metadata = tokio::fs::metadata(&canonical_target).await?;
    if !metadata.is_file() {
        return Ok((StatusCode::BAD_REQUEST, "Not a file").into_response());
    }

    let file = tokio::fs::File::open(&canonical_target).await?;
    let mut buffer = Vec::new();
    file.take((MAX_PREVIEW_BYTES + 1) as u64)
        .read_to_end(&mut buffer)
        .await?;

    let truncated = buffer.len() > MAX_PREVIEW_BYTES;
    buffer.truncate(MAX_PREVIEW_BYTES);

    // A NUL byte is a good enough signal that this isn't a text file.
    if buffer.contains(&0) {
        return Ok((StatusCode::UNSUPPORTED_MEDIA_TYPE, "Binary file").into_response());
    }

    let content = String::from_utf8_lossy(&buffer).into_owned();

    Ok(Json(WorkspaceFileResponse {
        path: relative.to_string_lossy().to_string(),
        content,
        truncated,
    })
    .into_response())
}
