use anyhow::Result;
use std::env;

use crate::google::custom_search::search_google;

/// Search the web and print results.
pub async fn run(query: String, limit: u32) -> Result<()> {
    let api_key = env::var("HQ_GOOGLE_SEARCH_API_KEY")?;
    let cx_id = env::var("HQ_GOOGLE_SEARCH_CX_ID")?;

    let results = search_google(&query, &api_key, &cx_id, Some(limit as u8), None).await?;

    for result in &results {
        println!("# {}", result.title);
        println!("{}", result.link);
        if !result.snippet.is_empty() {
            println!("{}", result.snippet);
        }
        println!();
    }
    Ok(())
}
