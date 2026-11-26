//! Public types for the chat API
use hq::openai::Message;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Clone)]
pub struct ChatSession {
    pub id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Deserialize)]
pub struct ChatRequest {
    pub session_id: String,
    pub message: String,
    /// Images uploaded to the session workspace that should be attached to
    /// this message. Files are referenced by their uploaded filename.
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ChatAttachment {
    pub filename: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

#[derive(Deserialize)]
pub struct ChatSessionsQuery {
    pub page: Option<usize>,
    pub limit: Option<usize>,
    // Use HTML form syntax "?tags=t1&tags=t2"
    pub tags: Option<Vec<String>>,
    // Exclude sessions containing any of these tags
    pub exclude_tags: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct ChatSessionsResponse {
    pub sessions: Vec<ChatSession>,
    pub page: usize,
    pub limit: usize,
    pub total_sessions: i64,
    pub total_pages: i64,
}

#[derive(Serialize)]
pub struct ChatResponse {
    message: String,
}

impl ChatResponse {
    pub fn new(message: &str) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Serialize)]
pub struct ChatTranscriptResponse {
    pub transcript: Vec<Message>,
    /// The session's generated title, once the background job has produced one.
    pub title: Option<String>,
    /// True when a response is currently being generated for this session, so
    /// the client should attach to the resumable stream.
    pub in_progress: bool,
}

#[derive(Deserialize)]
pub struct ChatStreamQuery {
    /// Resume cursor: replay buffered events with a greater id. Used when the
    /// browser has no `Last-Event-ID` (e.g. after a full page reload).
    pub after: Option<u64>,
}

#[derive(Deserialize)]
pub struct TerminalRequest {
    /// Shell command to run in the session's bashkit sandbox.
    pub command: String,
}

#[derive(Serialize)]
pub struct TerminalCwdResponse {
    pub cwd: String,
}

#[derive(Serialize)]
pub struct TerminalResponse {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    /// True when stdout or stderr hit bashkit's output limit.
    pub truncated: bool,
    /// Working directory inside the sandbox after the command, e.g. `/files`.
    pub cwd: String,
}

#[derive(Serialize)]
pub struct WorkspaceFilesResponse {
    pub files: Vec<WorkspaceFileEntry>,
}

#[derive(Serialize)]
pub struct WorkspaceFileEntry {
    /// Path relative to the session workspace, e.g. `notes/todo.md`.
    pub path: String,
    pub is_directory: bool,
}

#[derive(Serialize)]
pub struct WorkspaceFileResponse {
    pub path: String,
    pub content: String,
    /// True when the file was larger than the preview limit.
    pub truncated: bool,
}
