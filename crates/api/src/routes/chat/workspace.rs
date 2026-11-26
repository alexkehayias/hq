//! Session workspace files: list, preview, and upload into what a session owns.
//!
//! The terminal and the agent's bash tool both operate on
//! `{storage}/workspace/{workspace_id}`, where a loop run's transcript session
//! resolves to its loop's workspace id. These handlers expose that directory to
//! the chat UI's workspace panel without letting a request escape it.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::{
    Json,
    extract::{Multipart, Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::public::{
    WorkspaceFileEntry, WorkspaceFileResponse, WorkspaceFilesResponse, WorkspaceUploadResponse,
    WorkspaceUploadedFile,
};
use crate::public::ApiError;
use crate::state::AppState;
use hq::ai::chat::db::session_exists;
use hq::ai::chat::models::is_valid_session_id;
use hq::ai::chat::workspace::workspace_root;
use hq::loops::db::workspace_id_for_session;

type SharedState = Arc<RwLock<AppState>>;

/// Maximum number of entries returned for one listing.
const MAX_ENTRIES: usize = 2_000;

/// Maximum directory depth walked when listing.
const MAX_DEPTH: usize = 8;

/// Maximum bytes returned when previewing a file.
const MAX_PREVIEW_BYTES: usize = 256 * 1024;

/// Maximum size of a single workspace upload request.
pub const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// Directory inside the workspace where uploads are stored, shared with the
/// chat composer's image attachments. It is a plain directory, so the
/// terminal and agent tools see the same files.
const FILES_DIR: &str = "files";

/// A file field buffered from a multipart body, not yet validated.
struct PendingUpload {
    filename: String,
    bytes: axum::body::Bytes,
}

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

/// Reduce a client-supplied upload filename to a safe basename. Strips any
/// directory components and replaces separators/control characters so the
/// result cannot escape the upload directory.
fn sanitize_filename(raw: &str) -> Option<String> {
    let base = Path::new(raw).file_name()?.to_str()?;
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return None;
    }
    Some(cleaned)
}

/// Whether `bytes` look like text. Uploads are restricted to text/code so
/// every accepted file can be previewed in the panel; a NUL byte or invalid
/// UTF-8 marks a binary.
fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Write `bytes` into `dir`, choosing `name-1.ext` style names when the file
/// already exists. `create_new` claims each candidate atomically, so two
/// concurrent uploads cannot overwrite each other.
async fn write_unique(dir: &Path, filename: &str, bytes: &[u8]) -> std::io::Result<String> {
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(filename);
    let extension = path.extension().and_then(|extension| extension.to_str());

    for attempt in 0..1000 {
        let candidate = match (attempt, extension) {
            (0, _) => filename.to_string(),
            (n, Some(extension)) => format!("{stem}-{n}.{extension}"),
            (n, None) => format!("{stem}-{n}"),
        };

        let target = dir.join(&candidate);
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .await
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(bytes).await {
                    // Don't leave a partial file behind for the panel to list.
                    drop(file);
                    let _ = tokio::fs::remove_file(&target).await;
                    return Err(e);
                }
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "too many files with the same name",
    ))
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

    // A loop run's transcript session is keyed by the run id, but its files
    // live in the loop's workspace (see `workspace_id_for_session`).
    let workspace_id = workspace_id_for_session(&db, &session_id).await?;
    let root = workspace_root(&storage_path, &workspace_id);

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

    let workspace_id = workspace_id_for_session(&db, &session_id).await?;
    let root = workspace_root(&storage_path, &workspace_id);
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

/// Upload text/code files into the session workspace's `files/` directory.
///
/// The workspace panel's file picker posts here. Only text content is
/// accepted so every upload can be previewed in the panel; binaries are
/// rejected before anything is written.
pub async fn upload_workspace_files(
    State(state): State<SharedState>,
    AxumPath(session_id): AxumPath<String>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    if !is_valid_session_id(&session_id) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
    }

    let (db, storage_path) = {
        let state = state.read().expect("Unable to read shared state");
        (state.db.clone(), state.config.storage_path.clone())
    };

    // Check the session before reading the body so a bad request doesn't
    // buffer a large upload just to be rejected.
    if !session_exists(&db, &session_id).await? {
        return Ok((StatusCode::NOT_FOUND, "Session not found").into_response());
    }

    let mut pending = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(e) => {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    format!("Invalid multipart body: {e}"),
                )
                    .into_response());
            }
        };

        if field.name() != Some("file") {
            continue;
        }

        let filename = field.file_name().unwrap_or_default().to_string();
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                return Ok(
                    (StatusCode::BAD_REQUEST, format!("Invalid file field: {e}")).into_response(),
                );
            }
        };
        pending.push(PendingUpload { filename, bytes });
    }

    if pending.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "No file provided").into_response());
    }

    // Validate every file before writing any of them so a bad pick in the
    // batch does not leave a partial upload behind.
    let mut validated = Vec::with_capacity(pending.len());
    for upload in pending {
        let Some(filename) = sanitize_filename(&upload.filename) else {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("Invalid filename: {}", upload.filename),
            )
                .into_response());
        };
        // The file listing hides dotfiles, so an upload under a hidden name
        // would silently disappear from the panel.
        if filename.starts_with('.') {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("Hidden filenames aren't supported: {filename}"),
            )
                .into_response());
        }
        if !is_text(&upload.bytes) {
            return Ok((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("{filename} is not a text file"),
            )
                .into_response());
        }
        validated.push((filename, upload.bytes));
    }

    let workspace_id = workspace_id_for_session(&db, &session_id).await?;
    let files_dir = workspace_root(&storage_path, &workspace_id).join(FILES_DIR);
    tokio::fs::create_dir_all(&files_dir).await?;

    let mut files = Vec::with_capacity(validated.len());
    for (filename, bytes) in validated {
        let stored = write_unique(&files_dir, &filename, &bytes).await?;
        files.push(WorkspaceUploadedFile {
            path: format!("{FILES_DIR}/{stored}"),
            sandbox_path: format!("/{FILES_DIR}/{stored}"),
            size: bytes.len() as u64,
        });
    }

    Ok(Json(WorkspaceUploadResponse { files }).into_response())
}
