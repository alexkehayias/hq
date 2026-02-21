//! Router for the notes API

use std::sync::{Arc, RwLock};

use axum::{
    Router,
    extract::{Path, State},
    routing::{get, post},
};
use axum_extra::extract::Query;
use serde_json::{Value, json};

use super::public;
use crate::api::routes::notes::db as notes_db;
use crate::api::state::AppState;
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
) -> Result<axum::Json<public::ViewNoteResponse>, crate::api::public::ApiError> {
    let db = state.read().unwrap().db.clone();
    let note_result = notes_db::get_note_by_id(&db, id).await?;
    Ok(axum::Json(note_result))
}

/// Create the notes router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/search", get(note_search))
        .route("/index", post(index_notes))
        .route("/{id}/view", get(view_note))
}
