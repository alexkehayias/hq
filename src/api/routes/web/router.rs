//! Router for the web API

use std::sync::{Arc, RwLock};

use axum::{Router, extract::State, response::Json};
use axum_extra::extract::Query;

use super::public;
use crate::api::routes::web::public::{WebSearchResponse, WebSearchResult};
use crate::api::state::AppState;
use crate::core::AppConfig;
use crate::google::custom_search::search_google;

type SharedState = Arc<RwLock<AppState>>;

async fn web_search(
    State(state): State<SharedState>,
    Query(params): Query<public::WebSearchParams>,
) -> Result<Json<WebSearchResponse>, crate::api::public::ApiError> {
    let (api_key, cx_id) = {
        let shared_state = state.read().expect("Unable to read share state");
        let AppConfig {
            google_search_api_key,
            google_search_cx_id,
            ..
        } = &shared_state.config;
        (google_search_api_key.clone(), google_search_cx_id.clone())
    };

    let items = search_google(&params.query, &api_key, &cx_id, Some(params.limit), None).await?;

    let results: Vec<WebSearchResult> = items
        .into_iter()
        .map(|item| WebSearchResult {
            title: item.title,
            link: item.link,
            snippet: item.snippet,
        })
        .collect();

    let resp = WebSearchResponse {
        query: params.query.clone(),
        results,
    };
    Ok(Json(resp))
}

/// Create the web router
pub fn router() -> Router<SharedState> {
    Router::new().route("/search", axum::routing::get(web_search))
}
