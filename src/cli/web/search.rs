use anyhow::Result;

use crate::google::custom_search::search_google;

/// Search the web and print results.
pub async fn run(query: String, limit: u32, api_key: String, cx_id: String) -> Result<()> {
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
