//! Router for the file uploads API

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tokio::io::AsyncWriteExt;

use super::public::{FileUploadResponse, UploadedFile};
use crate::public::ApiError;
use crate::state::AppState;
use hq::ai::chat::models::is_valid_session_id;
use hq::ai::chat::workspace::workspace_root;
use hq::loops::db::workspace_id_for_session;

type SharedState = Arc<RwLock<AppState>>;

/// Maximum size of a single upload request.
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// Image extensions accepted as chat attachments.
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// Content type reported for text/code workspace uploads.
const TEXT_CONTENT_TYPE: &str = "text/plain; charset=utf-8";

/// A file field buffered from the multipart body, not yet validated.
struct PendingFile {
    filename: String,
    bytes: axum::body::Bytes,
}

/// Fields shared by both upload endpoints.
struct UploadFields {
    session_id: Option<String>,
    files: Vec<PendingFile>,
}

/// Reduce a client-supplied filename to a safe basename. Strips any directory
/// components and replaces separators/control characters so the result cannot
/// escape the target directory.
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

/// Lowercased extension of `filename`, if any.
fn extension_of(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

fn content_type_for(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Whether `bytes` look like text. Workspace uploads are restricted to
/// text/code so every accepted file can be previewed in the panel; a NUL byte
/// or invalid UTF-8 marks a binary.
fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Read the `session_id` and `file` fields shared by both upload endpoints.
async fn read_upload_fields(mut multipart: Multipart) -> Result<UploadFields, String> {
    let mut session_id = None;
    let mut files = Vec::new();

    loop {
        let field = multipart
            .next_field()
            .await
            .map_err(|e| format!("Invalid multipart body: {e}"))?;
        let Some(field) = field else { break };

        match field.name() {
            Some("session_id") => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| format!("Invalid session_id field: {e}"))?;
                if session_id.is_none() {
                    session_id = Some(value);
                }
            }
            Some("file") => {
                let filename = field.file_name().unwrap_or_default().to_string();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| format!("Invalid file field: {e}"))?;
                files.push(PendingFile { filename, bytes });
            }
            // Ignore unknown fields.
            _ => {}
        }
    }

    Ok(UploadFields { session_id, files })
}

/// Errors from resolving an upload's target workspace.
enum UploadTargetError {
    /// The session id was missing or unsafe.
    BadRequest(&'static str),
    /// The workspace lookup failed.
    Internal(ApiError),
}

impl IntoResponse for UploadTargetError {
    fn into_response(self) -> Response {
        match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            Self::Internal(e) => e.into_response(),
        }
    }
}

/// Validate the session id and resolve the workspace's `files/` directory.
///
/// Both upload endpoints take the session id from the multipart body. A loop
/// run's transcript session is keyed by the run id, but its files live in the
/// loop's workspace (see `workspace_id_for_session`).
async fn files_dir_for_upload(
    state: &SharedState,
    session_id: Option<String>,
) -> Result<PathBuf, UploadTargetError> {
    let session_id = match session_id {
        Some(id) if is_valid_session_id(&id) => id,
        Some(_) => return Err(UploadTargetError::BadRequest("Invalid session_id")),
        None => return Err(UploadTargetError::BadRequest("Missing session_id field")),
    };

    let (storage_path, db) = {
        let state = state.read().expect("Unable to read shared state");
        (state.config.storage_path.clone(), state.db.clone())
    };

    let workspace_id = workspace_id_for_session(&db, &session_id)
        .await
        .map_err(|e| UploadTargetError::Internal(ApiError::from(e)))?;

    Ok(workspace_root(&storage_path, &workspace_id).join("files"))
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
                // Tokio's `File` buffers writes in a background task, so
                // `write_all` can return before the bytes reach the file.
                // Flush before responding so a caller that reads the file
                // immediately doesn't see it empty.
                let result = async {
                    file.write_all(bytes).await?;
                    file.flush().await
                }
                .await;

                if let Err(e) = result {
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

/// Upload one or more images into the session workspace `files/` directory.
async fn upload_files(
    State(state): State<SharedState>,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    let UploadFields {
        session_id,
        files: pending,
    } = match read_upload_fields(multipart).await {
        Ok(fields) => fields,
        Err(message) => return Ok((StatusCode::BAD_REQUEST, message).into_response()),
    };

    let files_dir = match files_dir_for_upload(&state, session_id).await {
        Ok(dir) => dir,
        Err(e) => return Ok(e.into_response()),
    };

    if pending.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "No file provided").into_response());
    }

    // Validate every file before writing any of them so a bad upload in the
    // batch does not leave a partial result.
    let mut validated: Vec<(String, String, axum::body::Bytes)> = Vec::with_capacity(pending.len());
    for file in pending {
        let Some(filename) = sanitize_filename(&file.filename) else {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("Invalid filename: {}", file.filename),
            )
                .into_response());
        };
        let Some(ext) = extension_of(&filename) else {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("File has no extension: {filename}"),
            )
                .into_response());
        };
        if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("Unsupported file type: {ext}"),
            )
                .into_response());
        }
        validated.push((filename, ext, file.bytes));
    }

    tokio::fs::create_dir_all(&files_dir).await?;

    let mut uploaded = Vec::with_capacity(validated.len());
    for (filename, ext, bytes) in validated {
        tokio::fs::write(files_dir.join(&filename), &bytes).await?;
        let path = format!("files/{filename}");
        uploaded.push(UploadedFile {
            filename,
            sandbox_path: format!("/{path}"),
            path,
            size: bytes.len() as u64,
            content_type: content_type_for(&ext).to_string(),
        });
    }

    Ok(Json(FileUploadResponse { files: uploaded }).into_response())
}

/// Upload text/code files into the session workspace `files/` directory.
///
/// The workspace panel's file picker posts here. Only text content is
/// accepted so every upload can be previewed in the panel; binaries are
/// rejected before anything is written. Unlike attachments, a name collision
/// keeps both files by appending `-1`, `-2`, and so on.
async fn upload_workspace_files(
    State(state): State<SharedState>,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    let UploadFields {
        session_id,
        files: pending,
    } = match read_upload_fields(multipart).await {
        Ok(fields) => fields,
        Err(message) => return Ok((StatusCode::BAD_REQUEST, message).into_response()),
    };

    let files_dir = match files_dir_for_upload(&state, session_id).await {
        Ok(dir) => dir,
        Err(e) => return Ok(e.into_response()),
    };

    if pending.is_empty() {
        return Ok((StatusCode::BAD_REQUEST, "No file provided").into_response());
    }

    // Validate every file before writing any of them so a bad pick in the
    // batch does not leave a partial upload behind.
    let mut validated = Vec::with_capacity(pending.len());
    for file in pending {
        let Some(filename) = sanitize_filename(&file.filename) else {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("Invalid filename: {}", file.filename),
            )
                .into_response());
        };
        // The workspace listing hides dotfiles, so an upload under a hidden
        // name would silently disappear from the panel.
        if filename.starts_with('.') {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("Hidden filenames aren't supported: {filename}"),
            )
                .into_response());
        }
        if !is_text(&file.bytes) {
            return Ok((
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("{filename} is not a text file"),
            )
                .into_response());
        }
        validated.push((filename, file.bytes));
    }

    tokio::fs::create_dir_all(&files_dir).await?;

    let mut uploaded = Vec::with_capacity(validated.len());
    for (filename, bytes) in validated {
        let stored = write_unique(&files_dir, &filename, &bytes).await?;
        let path = format!("files/{stored}");
        uploaded.push(UploadedFile {
            filename: stored,
            sandbox_path: format!("/{path}"),
            path,
            size: bytes.len() as u64,
            content_type: TEXT_CONTENT_TYPE.to_string(),
        });
    }

    Ok(Json(FileUploadResponse { files: uploaded }).into_response())
}

/// Serve a file previously uploaded into the session workspace `files/`
/// directory. Used by the chat UI to render attachments when replaying a
/// transcript.
async fn get_file(
    State(state): State<SharedState>,
    AxumPath((session_id, filename)): AxumPath<(String, String)>,
) -> Result<Response, ApiError> {
    if !is_valid_session_id(&session_id) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
    }

    let Some(filename) = sanitize_filename(&filename) else {
        return Ok((StatusCode::BAD_REQUEST, "Invalid filename").into_response());
    };

    let ext = extension_of(&filename).unwrap_or_default();
    if !IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return Ok((StatusCode::BAD_REQUEST, "Unsupported file type").into_response());
    }

    let (storage_path, db) = {
        let state = state.read().expect("Unable to read shared state");
        (state.config.storage_path.clone(), state.db.clone())
    };

    let workspace_id = workspace_id_for_session(&db, &session_id).await?;

    let path = workspace_root(&storage_path, &workspace_id)
        .join("files")
        .join(&filename);

    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok((
            [(axum::http::header::CONTENT_TYPE, content_type_for(&ext))],
            bytes,
        )
            .into_response()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((StatusCode::NOT_FOUND, "File not found").into_response())
        }
        Err(e) => Err(e.into()),
    }
}

/// Delete a file previously uploaded into the session workspace `files/`
/// directory. The session ID is part of the path so a delete is always scoped
/// to a single session and cannot reach another session's uploads.
async fn delete_file(
    State(state): State<SharedState>,
    AxumPath((session_id, filename)): AxumPath<(String, String)>,
) -> Result<Response, ApiError> {
    if !is_valid_session_id(&session_id) {
        return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
    }

    let Some(filename) = sanitize_filename(&filename) else {
        return Ok((StatusCode::BAD_REQUEST, "Invalid filename").into_response());
    };

    let (storage_path, db) = {
        let state = state.read().expect("Unable to read shared state");
        (state.config.storage_path.clone(), state.db.clone())
    };

    let workspace_id = workspace_id_for_session(&db, &session_id).await?;

    let path = workspace_root(&storage_path, &workspace_id)
        .join("files")
        .join(&filename);

    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((StatusCode::NOT_FOUND, "File not found").into_response())
        }
        Err(e) => Err(e.into()),
    }
}

/// Create the file uploads router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", post(upload_files))
        .route("/workspace", post(upload_workspace_files))
        .route(
            "/{session_id}/{filename}",
            get(get_file).delete(delete_file),
        )
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
}
