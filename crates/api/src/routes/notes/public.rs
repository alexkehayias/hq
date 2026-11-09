//! Public types for the notes API
use serde::{Deserialize, Serialize};

// The search result shapes are produced by the library, so they live there and
// are re-exported here for API consumers.
pub use hq::types::notes::{SearchResponse, SearchResult};

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

#[derive(Serialize)]
pub struct ViewNoteResponse {
    pub id: String,
    pub title: String,
    pub body: String,
    pub tags: Option<String>,
    pub r#type: Option<String>,
    pub status: Option<String>,
    pub file_name: String,
}

#[derive(Deserialize)]
pub struct UpdateNoteRequest {
    pub status: String,
}
