//! Public types for the notes API
use serde::{Deserialize, Serialize};

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
