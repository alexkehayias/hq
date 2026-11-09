//! Public types for the file uploads API
use serde::Serialize;

#[derive(Serialize)]
pub struct FileUploadResponse {
    pub files: Vec<UploadedFile>,
}

#[derive(Serialize)]
pub struct UploadedFile {
    /// Sanitized basename the file was stored under.
    pub filename: String,
    /// Path relative to the session workspace, e.g. `files/screenshot.png`.
    pub path: String,
    /// Path as seen by the agent's bash sandbox, e.g. `/files/screenshot.png`.
    pub sandbox_path: String,
    pub size: u64,
    pub content_type: String,
}
