//! Router for the webhook API

use axum::{Json, Router, extract::State, http::StatusCode};
use std::sync::{Arc, RwLock};

use super::public::{BlurtNotification, GithubPushEvent, Notification};
use crate::api::state::AppState;

type SharedState = Arc<RwLock<AppState>>;

/// Handle forwarded desktop notifications from daemon
async fn blurt_webhook(Json(notification): Json<BlurtNotification>) -> StatusCode {
    tracing::info!("Received Blurt notification: {:?}", notification);
    StatusCode::OK
}

/// Handle GitHub push webhooks for the notes repo. The webhook proxy in front
/// of this server authenticates GitHub's signature, so we don't verify it here.
async fn github_push(
    State(state): State<SharedState>,
    Json(event): Json<GithubPushEvent>,
) -> StatusCode {
    if event.git_ref != "refs/heads/main" {
        tracing::debug!("github_push: ignoring push to {}", event.git_ref);
        return StatusCode::OK;
    }

    tracing::info!("github_push: main updated, reindexing changed notes");

    let (config, db) = {
        let shared_state = state.read().expect("Unable to read shared state");
        (shared_state.config.clone(), shared_state.db.clone())
    };
    tokio::spawn(async move {
        crate::search::sync_and_reindex_notes(&db, &config).await;
    });

    StatusCode::OK
}

/// Handle forwarded iOS notifications
async fn notification_webhook(Json(notification): Json<Notification>) -> StatusCode {
    tracing::info!("Received iOS notification: {:?}", notification);
    StatusCode::OK
}

/// Create the webhook router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/blurt", axum::routing::post(blurt_webhook))
        .route("/github", axum::routing::post(github_push))
        .route("/notification", axum::routing::post(notification_webhook))
}
