//! Public types for the loops API

use serde::Serialize;

#[derive(Serialize)]
pub struct LoopListResponse {
    pub loops: Vec<LoopSummaryResponse>,
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
