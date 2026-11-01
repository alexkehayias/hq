//! Router for the loops API

use std::sync::{Arc, RwLock};

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{get, put},
};
use serde_json::json;

use super::public::{
    LoopListResponse, LoopRunResponse, LoopRunsQuery, LoopRunsResponse, LoopSummaryResponse,
    UpdateLoopRequest, UpdateLoopResponse,
};
use crate::api::public::ApiError;
use crate::api::state::AppState;
use crate::core::AppConfig;
use crate::loops::db::{find_all_loops, find_loop, find_loop_runs, update_loop_system_prompt};
use crate::loops::supervisor::{self, LoopSupervisorConfig};

type SharedState = Arc<RwLock<AppState>>;

/// List every loop with its subscribed channels and running status.
async fn list_loops(State(state): State<SharedState>) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .read()
        .expect("Unable to read shared state")
        .db
        .clone();

    let loops = find_all_loops(&db).await?;
    let running = supervisor::running_ids();

    let loops = loops
        .into_iter()
        .map(|record| LoopSummaryResponse {
            running: running.iter().any(|id| id == &record.id),
            id: record.id,
            subscribe_channels: record.subscribe_channels,
            emit_channels: record.emit_channels,
            system_prompt: record.system_prompt,
            tools: record.tools,
            created_at: record.created_at,
        })
        .collect();

    Ok(Json(LoopListResponse { loops }))
}

/// Replace a loop's system prompt, restarting it if it is currently running so
/// the new prompt applies to subsequent events.
async fn update_loop(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateLoopRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.system_prompt.trim().is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "System prompt cannot be empty"})),
        )
            .into_response());
    }

    let (db, loop_config) = {
        let state = state.read().expect("Unable to read shared state");
        let AppConfig {
            note_search_api_url,
            storage_path,
            openai_api_hostname,
            openai_api_key,
            openai_model,
            vapid_key_path,
            ..
        } = &state.config;

        let config = LoopSupervisorConfig {
            db: state.db.clone(),
            api_base_url: note_search_api_url.clone(),
            api_hostname: openai_api_hostname.clone(),
            api_key: openai_api_key.clone(),
            model: openai_model.clone(),
            storage_path: storage_path.clone(),
            vapid_key_path: vapid_key_path.clone(),
        };
        (state.db.clone(), config)
    };

    let updated = update_loop_system_prompt(&db, &id, &body.system_prompt).await?;
    if updated == 0 {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Loop '{}' not found", id)})),
        )
            .into_response());
    }

    // The running task holds its own copy of the prompt, so restart it to pick
    // up the change. A failed restart is logged but does not undo the save —
    // the loop respawns from the DB on the next server boot.
    if supervisor::running_ids()
        .iter()
        .any(|running| running == &id)
        && let Some(record) = find_loop(&db, &id).await?
        && let Err(e) = supervisor::start(record, loop_config).await
    {
        tracing::warn!("failed to restart loop '{}' after prompt update: {}", id, e);
    }

    Ok(Json(UpdateLoopResponse { success: true }).into_response())
}

/// Recent runs for a loop, newest first.
async fn list_loop_runs(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(params): Query<LoopRunsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .read()
        .expect("Unable to read shared state")
        .db
        .clone();

    if find_loop(&db, &id).await?.is_none() {
        return Ok((StatusCode::NOT_FOUND, "Loop not found").into_response());
    }

    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let runs = find_loop_runs(&db, &id, limit)
        .await?
        .into_iter()
        .map(|run| LoopRunResponse {
            id: run.id,
            loop_id: run.loop_id,
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
        })
        .collect();

    Ok(Json(LoopRunsResponse { runs }).into_response())
}

/// Create the loops router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", get(list_loops))
        .route("/{id}", put(update_loop))
        .route("/{id}/runs", get(list_loop_runs))
}
