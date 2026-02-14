//! Router for the kv API (latest selection storage)

use std::sync::{Arc, RwLock};

use axum::{Json, Router, extract::State};
use serde_json::Value;

use crate::api::state::{AppState, LastSelection};

type SharedState = Arc<RwLock<AppState>>;

async fn kv_get(State(state): State<SharedState>) -> Json<Option<Value>> {
    if let Some(LastSelection {
        id,
        file_name,
        title,
    }) = &state.read().unwrap().latest_selection
    {
        let resp = serde_json::json!({
            "id": id,
            "file_name": file_name,
            "title": title,
        });
        Json(Some(resp))
    } else {
        Json(None)
    }
}

async fn kv_set(State(state): State<SharedState>, Json(data): Json<LastSelection>) {
    state.write().unwrap().latest_selection = Some(LastSelection {
        id: data.id,
        file_name: data.file_name,
        title: data.title,
    });
}

/// Create the kv router
pub fn router() -> Router<SharedState> {
    Router::new().route("/latest", axum::routing::get(kv_get).post(kv_set))
}
