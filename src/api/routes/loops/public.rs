//! Public types for the loops API

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct LoopListResponse {
    pub loops: Vec<LoopSummaryResponse>,
}

#[derive(Deserialize)]
pub struct UpdateLoopRequest {
    pub system_prompt: String,
}

#[derive(Serialize)]
pub struct UpdateLoopResponse {
    pub success: bool,
}

#[derive(Serialize)]
pub struct LoopSummaryResponse {
    pub id: String,
    pub subscribe_channels: Vec<String>,
    pub emit_channels: Vec<String>,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub created_at: String,
    pub running: bool,
}

#[derive(Deserialize)]
pub struct LoopRunsQuery {
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct LoopRunsResponse {
    pub runs: Vec<LoopRunResponse>,
}

#[derive(Serialize)]
pub struct LoopRunResponse {
    pub id: String,
    pub loop_id: String,
    pub channel_id: String,
    pub event: String,
    pub status: String,
    pub session_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub response: Option<String>,
    pub error: Option<String>,
    pub has_transcript: bool,
}
