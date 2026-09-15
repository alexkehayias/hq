//! Router for the webhook API

use axum::{Json, Router, http::StatusCode};
use std::sync::{Arc, RwLock};

use super::public::{BlurtNotification, Notification};
use crate::api::state::AppState;

type SharedState = Arc<RwLock<AppState>>;

/// Handle forwarded desktop notifications from daemon
async fn blurt_webhook(Json(notification): Json<BlurtNotification>) -> StatusCode {
    tracing::info!("Received Blurt notification: {:?}", notification);
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
        .route("/notification", axum::routing::post(notification_webhook))
}
