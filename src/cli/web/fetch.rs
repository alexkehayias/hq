use anyhow::Result;

use crate::core::http::fetch_url_to_markdown;

/// Fetch a URL and print its content as markdown to stdout.
pub async fn run(url: String) -> Result<()> {
    let markdown = fetch_url_to_markdown(&url).await?;
    println!("{}", markdown);
    Ok(())
}
