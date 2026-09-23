//! Router for the file uploads API

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};

use super::public::{FileUploadResponse, UploadedFile};
use crate::api::public::ApiError;
use crate::api::state::AppState;

type SharedState = Arc<RwLock<AppState>>;

/// Maximum size of a single upload request.
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// File extensions accepted by the upload endpoint.
const ALLOWED_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "pdf"];

/// A file field buffered from the multipart body, not yet validated.
struct PendingFile {
    filename: String,
    bytes: axum::body::Bytes,
}

/// Session IDs are used as a path segment (`workspace/{session_id}`), so
/// restrict them to a safe character set to prevent path traversal.
fn is_valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
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
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Upload one or more images/PDFs into the session workspace `files/` directory.
async fn upload_files(
    State(state): State<SharedState>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    let storage_path = state
        .read()
        .expect("Unable to read shared state")
        .config
        .storage_path
        .clone();

    let mut session_id: Option<String> = None;
    let mut pending: Vec<PendingFile> = Vec::new();

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

        let name = field.name().map(|n| n.to_string());
        match name.as_deref() {
            Some("session_id") => {
                let value = match field.text().await {
                    Ok(value) => value,
                    Err(e) => {
                        return Ok((
                            StatusCode::BAD_REQUEST,
                            format!("Invalid session_id field: {e}"),
                        )
                            .into_response());
                    }
                };
                if session_id.is_none() {
                    session_id = Some(value);
                }
            }
            Some("file") => {
                let filename = field.file_name().unwrap_or_default().to_string();
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        return Ok(
                            (StatusCode::BAD_REQUEST, format!("Invalid file field: {e}"))
                                .into_response(),
                        );
                    }
                };
                pending.push(PendingFile { filename, bytes });
            }
            // Ignore unknown fields.
            _ => {}
        }
    }

    let session_id = match session_id {
        Some(id) if is_valid_session_id(&id) => id,
        Some(_) => {
            return Ok((StatusCode::BAD_REQUEST, "Invalid session_id").into_response());
        }
        None => {
            return Ok((StatusCode::BAD_REQUEST, "Missing session_id field").into_response());
        }
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
        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            return Ok((
                StatusCode::BAD_REQUEST,
                format!("Unsupported file type: {ext}"),
            )
                .into_response());
        }
        validated.push((filename, ext, file.bytes));
    }

    let files_dir = PathBuf::from(&storage_path)
        .join("workspace")
        .join(&session_id)
        .join("files");
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

/// Create the file uploads router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", post(upload_files))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
}
