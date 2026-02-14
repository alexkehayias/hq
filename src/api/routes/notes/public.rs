//! Public types for the notes API
use serde::{Deserialize, Serialize};
use crate::openai::Message;

// Search

fn default_limit() -> usize {
    20
}

fn default_as_true() -> bool {
    true
}

fn default_as_false() -> bool {
    false
}

#[derive(Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_as_false")]
    pub include_similarity: bool,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_as_true")]
    pub truncate: bool,
}

#[derive(Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub r#type: String,
    pub title: String,
    pub category: String,
    pub file_name: String,
    pub tags: Option<String>,
    pub is_task: bool,
    pub task_status: Option<String>,
    pub task_scheduled: Option<String>,
    pub task_deadline: Option<String>,
    pub task_closed: Option<String>,
    pub meeting_date: Option<String>,
    pub body: String,
}

#[derive(Serialize, Deserialize)]
pub struct SearchResponse {
    pub raw_query: String,
    pub parsed_query: String,
    pub results: Vec<SearchResult>,
}

#[derive(Serialize)]
pub struct ViewNoteResponse {
    pub id: String,
    pub title: String,
    pub body: String,
    pub tags: Option<String>,
}

// Chat

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
}
