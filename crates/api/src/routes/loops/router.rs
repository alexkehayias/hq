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
    LoopListResponse, LoopRunResponse, LoopRunsPageQuery, LoopRunsPageResponse, LoopRunsQuery,
    LoopRunsResponse, LoopSummaryResponse, UpdateLoopRequest, UpdateLoopResponse,
};
use crate::public::ApiError;
use crate::state::AppState;
use hq::core::AppConfig;
use hq::loops::db::{
    count_loop_runs, find_all_loops, find_loop, find_loop_runs, find_loop_runs_page,
    update_loop_system_prompt, update_loop_title,
};
use hq::loops::supervisor::{self, LoopSupervisorConfig};

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
            title: record.title,
            subscribe_channels: record.subscribe_channels,
            emit_channels: record.emit_channels,
            system_prompt: record.system_prompt,
            tools: record.tools,
            created_at: record.created_at,
        })
        .collect();

    Ok(Json(LoopListResponse { loops }))
}

/// Update a loop's title and/or system prompt. Restarts the loop when its
/// prompt changes so the new prompt applies to subsequent events.
async fn update_loop(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateLoopRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let title = body.title.map(|title| title.trim().to_string());
    let system_prompt = body.system_prompt;

    if title.is_none() && system_prompt.is_none() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Nothing to update"})),
        )
            .into_response());
    }

    if title.as_deref().is_some_and(str::is_empty) {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Title cannot be empty"})),
        )
            .into_response());
    }

    if system_prompt
        .as_deref()
        .is_some_and(|prompt| prompt.trim().is_empty())
    {
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
            index_path,
            openai_api_hostname,
            openai_api_key,
            openai_model,
            openai_decision_model,
            vapid_key_path,
            ..
        } = &state.config;

        let config = LoopSupervisorConfig {
            db: state.db.clone(),
            api_base_url: note_search_api_url.clone(),
            api_hostname: openai_api_hostname.clone(),
            api_key: openai_api_key.clone(),
            model: openai_model.clone(),
            decision_model: openai_decision_model.clone(),
            storage_path: storage_path.clone(),
            index_path: index_path.clone(),
            vapid_key_path: vapid_key_path.clone(),
        };
        (state.db.clone(), config)
    };

    let not_found = || {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Loop '{}' not found", id)})),
        )
            .into_response()
    };

    if let Some(title) = &title
        && update_loop_title(&db, &id, title).await? == 0
    {
        return Ok(not_found());
    }

    if let Some(system_prompt) = &system_prompt
        && update_loop_system_prompt(&db, &id, system_prompt).await? == 0
    {
        return Ok(not_found());
    }

    // The running task holds its own copy of the prompt, so restart it to pick
    // up the change. A title change needs no restart. A failed restart is
    // logged but does not undo the save — the loop respawns from the DB on the
    // next server boot.
    if system_prompt.is_some()
        && supervisor::running_ids()
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
        .map(LoopRunResponse::from)
        .collect();

    Ok(Json(LoopRunsResponse { runs }).into_response())
}

/// A page of runs across every loop, newest first.
async fn list_all_loop_runs(
    State(state): State<SharedState>,
    Query(params): Query<LoopRunsPageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let db = state
        .read()
        .expect("Unable to read shared state")
        .db
        .clone();

    let page = params.page.unwrap_or(1).max(1);
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * limit;

    let total_runs = count_loop_runs(&db).await?;
    let runs = find_loop_runs_page(&db, limit, offset)
        .await?
        .into_iter()
        .map(LoopRunResponse::from)
        .collect();
    let total_pages = (total_runs as f64 / limit as f64).ceil() as i64;

    Ok(Json(LoopRunsPageResponse {
        runs,
        page,
        limit,
        total_runs,
        total_pages,
    })
    .into_response())
}

/// Create the loops router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", get(list_loops))
        .route("/runs", get(list_all_loop_runs))
        .route("/{id}", put(update_loop))
        .route("/{id}/runs", get(list_loop_runs))
}
