//! Router for the webhook API

use axum::{Json, Router, extract::State, http::StatusCode};
use std::sync::{Arc, RwLock};

use super::public::{BlurtNotification, GithubPushEvent};
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
    crate::api::routes::spawn_notes_sync(&state);

    StatusCode::OK
}

/// Create the webhook router (Blurt desktop notifications)
pub fn router() -> Router<SharedState> {
    Router::new().route("/blurt", axum::routing::post(blurt_webhook))
}

/// Create the GitHub webhook router
pub fn github_router() -> Router<SharedState> {
    Router::new().route("/github", axum::routing::post(github_push))
}
