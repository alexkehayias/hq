//! Public types for the web API
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct WebSearchParams {
    pub query: String,
    #[serde(default = "default_web_limit")]
    pub limit: u8,
}

fn default_web_limit() -> u8 {
    3
}

#[derive(Serialize, Deserialize)]
pub struct WebSearchResult {
    pub title: String,
    pub link: String,
    pub snippet: String,
}

#[derive(Serialize, Deserialize)]
pub struct WebSearchResponse {
    pub query: String,
    pub results: Vec<WebSearchResult>,
}
