//! API routes module

pub mod calendar;
pub mod chat;
pub mod email;
mod kv;
pub mod metrics;
pub mod notes;
pub mod push;
pub mod web;
pub mod webhook;

use std::sync::{Arc, RwLock};

use crate::api::state::AppState;
use axum::Router;

type SharedState = Arc<RwLock<AppState>>;

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
        // Webhook routes
        .nest("/webhook", webhook::router())
}
