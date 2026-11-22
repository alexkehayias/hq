//! Router for the webhook API

use axum::{Json, Router, extract::State, http::StatusCode};
use std::sync::{Arc, RwLock};
use tokio_rusqlite::rusqlite;

use super::public::{BlurtNotification, GithubPushEvent, LmProxyMetric, Notification};
use crate::public::ApiError;
use crate::state::AppState;
use hq::search::sync_and_reindex_notes;

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
        sync_and_reindex_notes(&db, &config).await;
    });

    StatusCode::OK
}

/// Handle forwarded iOS notifications
async fn notification_webhook(Json(notification): Json<Notification>) -> StatusCode {
    tracing::info!("Received iOS notification: {:?}", notification);
    StatusCode::OK
}

/// Handle token usage posted by lm-proxy after each proxied request. The
/// webhook proxy in front of this server authenticates the caller, so we
/// don't verify a signature here.
async fn lm_proxy_metric(
    State(state): State<SharedState>,
    Json(payload): Json<LmProxyMetric>,
) -> Result<StatusCode, ApiError> {
    let db = state.read().unwrap().db.clone();

    // Insert the metric event into the database. name/value stay NULL
    // for new rows — they only exist on legacy data migrated from the
    // pre-bucket schema.
    db.call(move |conn| {
        conn.execute(
            "INSERT INTO metric_event (input, output, cache_read, cache_write, reasoning) \
             VALUES (?, ?, ?, ?, ?)",
            tokio_rusqlite::params![
                payload.input,
                payload.output,
                payload.cache_read,
                payload.cache_write,
                payload.reasoning,
            ],
        )?;
        Ok::<_, rusqlite::Error>(())
    })
    .await?;

    Ok(StatusCode::OK)
}

/// Create the webhook router
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/blurt", axum::routing::post(blurt_webhook))
        .route("/github", axum::routing::post(github_push))
        .route("/lm-proxy", axum::routing::post(lm_proxy_metric))
        .route("/notification", axum::routing::post(notification_webhook))
}
