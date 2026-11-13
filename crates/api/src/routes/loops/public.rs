//! Public types for the loops API

use serde::{Deserialize, Serialize};

use hq::loops::models::LoopRun;

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
    pub title: String,
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

#[derive(Deserialize)]
pub struct LoopRunsPageQuery {
    pub page: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct LoopRunsPageResponse {
    pub runs: Vec<LoopRunResponse>,
    pub page: usize,
    pub limit: usize,
    pub total_runs: i64,
    pub total_pages: i64,
}

#[derive(Serialize)]
pub struct LoopRunResponse {
    pub id: String,
    pub loop_id: String,
    pub loop_title: String,
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

impl From<LoopRun> for LoopRunResponse {
    fn from(run: LoopRun) -> Self {
        Self {
            id: run.id,
            loop_id: run.loop_id,
            loop_title: run.loop_title,
            channel_id: run.channel_id,
            event: run.event,
            status: run.status.to_string(),
            session_id: run.session_id,
            started_at: run.started_at,
            finished_at: run.finished_at,
            duration_ms: run.duration_ms,
            response: run.response,
            error: run.error,
            has_transcript: run.has_transcript,
        }
    }
}
