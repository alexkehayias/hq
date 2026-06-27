use crate::core::db::async_db;
use crate::search::aql;
use crate::search::search_notes;
use anyhow::Result;
use serde_json::json;

pub async fn run(term: String, vector: bool, index_path: &str, vec_db_path: &str) -> Result<()> {
    let db = async_db(vec_db_path)
        .await
        .expect("Failed to connect to async db");
    let query = aql::parse_query(&term).expect("Parsing AQL failed");
    let cache_dir = std::env::var("HQ_FASTEMBED_CACHE_DIR")
        .unwrap_or_else(|_| ".fastembed_cache".to_string());
    let results = search_notes(index_path, &db, vector, false, &query, 20, &cache_dir).await?;
    println!(
        "{}",
        json!({
            "query": term,
            "results": results,
        })
    );
    Ok(())
}
