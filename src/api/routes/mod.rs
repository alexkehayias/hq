//! API routes module

pub mod calendar;
pub mod chat;
pub mod email;
mod kv;
pub mod metrics;
pub mod notes;
pub mod push;
pub mod skills;
pub mod web;
pub mod webhook;

use std::sync::{Arc, RwLock};

use crate::api::state::AppState;
use axum::Router;

type SharedState = Arc<RwLock<AppState>>;

/// Spawn a background notes sync + reindex. Shared by the `/notes/index`
/// endpoint and the GitHub push webhook, both of which return before the sync
/// completes.
pub(crate) fn spawn_notes_sync(state: &SharedState) {
    let (config, db) = {
        let shared_state = state.read().expect("Unable to read shared state");
        (shared_state.config.clone(), shared_state.db.clone())
    };
    tokio::spawn(async move {
        crate::jobs::sync_and_reindex_notes(&db, &config).await;
    });
}

/// Create the combined API router
pub fn router() -> Router<SharedState> {
    Router::new()
        // Notes routes
        .nest("/notes", notes::router())
        // Chat routes
        .nest("/chat", chat::router())
        // KV routes (for latest selection)
        .nest("/notes/search", kv::router())
        // Push notification routes
        .nest("/push", push::router())
        // Email routes
        .nest("/email", email::router())
        // Calendar routes
        .nest("/calendar", calendar::router())
        // Web search routes
        .nest("/web", web::router())
        // Metrics routes
        .nest("/metrics", metrics::router())
        // Skills routes
        .nest("/skills", skills::router())
        // Webhook routes
        .nest("/webhook", webhook::router())
}
