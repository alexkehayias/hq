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
