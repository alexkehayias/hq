//! Router for the webhook API

use axum::{Json, Router, http::StatusCode};
use std::sync::{Arc, RwLock};

use super::public::BlurtNotification;
use crate::api::state::AppState;

type SharedState = Arc<RwLock<AppState>>;

/// Handle forwarded desktop notifications from daemon
async fn blurt_webhook(Json(notification): Json<BlurtNotification>) -> StatusCode {
    tracing::info!("Received Blurt notification: {:?}", notification);
    StatusCode::OK
}

/// Create the webhook router
pub fn router() -> Router<SharedState> {
    Router::new().route("/blurt", axum::routing::post(blurt_webhook))
}
