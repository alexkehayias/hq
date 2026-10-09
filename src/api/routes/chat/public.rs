//! Public types for the chat API
use crate::openai::Message;
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
