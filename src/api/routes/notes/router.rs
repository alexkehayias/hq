//! Router for the notes API

use std::sync::{Arc, RwLock};

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
};
use axum_extra::extract::Query;
use serde_json::{Value, json};

use super::public;
use crate::api::routes::notes::db as notes_db;
use crate::api::state::AppState;
use crate::core::orgmode as core_org;
use crate::search::aql;
use crate::search::index_all;
use crate::search::search_notes;

type SharedState = Arc<RwLock<AppState>>;

// Note search endpoint
async fn note_search(
    State(state): State<SharedState>,
    Query(params): Query<public::SearchRequest>,
) -> Result<axum::Json<public::SearchResponse>, crate::api::public::ApiError> {
    let raw_query = params.query;
    let query = aql::parse_query(&raw_query).expect("Parsing AQL failed");
    let (db, index_path) = {
        let shared_state = state.read().unwrap();
        (
            shared_state.db.clone(),
            shared_state.config.index_path.clone(),
        )
    };

    let results = search_notes(
        &index_path,
        &db,
        params.include_similarity,
        params.truncate,
        &query,
        params.limit,
    )
    .await?;

    let resp = public::SearchResponse {
        raw_query: raw_query.to_string(),
        parsed_query: format!("{:?}", query),
        results,
    };

    Ok(axum::Json(resp))
}

// Index notes endpoint
async fn index_notes(
    State(state): State<SharedState>,
) -> Result<axum::Json<Value>, crate::api::public::ApiError> {
    let (a_db, index_path, notes_path, deploy_key_path) = {
        let shared_state = state.read().expect("Unable to read share state");
        (
            shared_state.db.clone(),
            shared_state.config.index_path.clone(),
            shared_state.config.notes_path.clone(),
            shared_state.config.deploy_key_path.clone(),
        )
    };
    tokio::spawn(async move {
        crate::core::git::maybe_pull_and_reset_repo(&deploy_key_path, &notes_path).await;
        let diff = crate::core::git::diff_last_commit_files(&deploy_key_path, &notes_path).await;
        let paths: Vec<std::path::PathBuf> = diff
            .iter()
            .map(|f| std::path::PathBuf::from(format!("{}/{}", &notes_path, f)))
            .collect();
        let filter_paths = if paths.is_empty() { None } else { Some(paths) };
        index_all(&a_db, &index_path, &notes_path, true, true, filter_paths)
            .await
            .unwrap();
    });
    Ok(axum::Json(json!({ "success": true })))
}

// View note endpoint
async fn view_note(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, crate::api::public::ApiError> {
    let db = state.read().unwrap().db.clone();
    match notes_db::get_note_by_id(&db, id).await {
        Ok(Some(note)) => Ok(axum::Json(note).into_response()),
        Ok(None) => Ok((StatusCode::NOT_FOUND, "Note not found").into_response()),
        Err(e) => Err(e.into()),
    }
}

// Update note status endpoint
async fn update_note(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<public::UpdateNoteRequest>,
) -> Result<axum::response::Response, crate::api::public::ApiError> {
    let (db, notes_path, index_path) = {
        let shared_state = state.read().unwrap();
        (
            shared_state.db.clone(),
            shared_state.config.notes_path.clone(),
            shared_state.config.index_path.clone(),
        )
    };

    let note = notes_db::get_note_by_id(&db, id.clone()).await?;
    let note = match note {
        Some(n) => n,
        None => return Ok((StatusCode::NOT_FOUND, "Note not found").into_response()),
    };

    if note.r#type.as_deref() != Some("task") {
        return Ok((
            StatusCode::BAD_REQUEST,
            "Only task notes can have their status updated",
        )
            .into_response());
    }

    let file_path = std::path::PathBuf::from(&notes_path).join(&note.file_name);
    core_org::update_task_in_file(&file_path, &id, None, None, Some(&body.status)).await?;

    // Re-index only the file that was modified
    index_all(&db, &index_path, &notes_path, true, true, Some(vec![file_path])).await?;

    // Re-fetch to return the indexed state
    match notes_db::get_note_by_id(&db, id).await? {
        Some(updated) => Ok(axum::Json(updated).into_response()),
        None => Ok((StatusCode::NOT_FOUND, "Note not found").into_response()),
    }
}

/// Create the notes router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/search", get(note_search))
        .route("/index", post(index_notes))
        .route("/{id}/view", get(view_note))
        .route("/{id}", patch(update_note))
}
