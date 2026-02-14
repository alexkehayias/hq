//! Router for the webhook API

use std::sync::{Arc, RwLock};
use axum::{Json, Router, http::StatusCode};

use crate::api::state::AppState;
use super::public::BlurtNotification;


type SharedState = Arc<RwLock<AppState>>;

/// Handle forwarded desktop notifications from daemon
async fn blurt_webhook(
    Json(notification): Json<BlurtNotification>,
) -> StatusCode {
    tracing::info!("Received Blurt notification: {:?}", notification);
    StatusCode::OK
}

/// Create the webhook router
pub fn router() -> Router<SharedState> {
    Router::new().route("/blurt", axum::routing::post(blurt_webhook))
}
