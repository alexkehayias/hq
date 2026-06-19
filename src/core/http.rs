use anyhow::Result;
use std::time::Duration;

/// Fetch a URL and convert its HTML content to markdown.
///
/// Shared core used by the CLI `web fetch` command. For callers that need
/// custom HTTP error handling (e.g. the AI `website_view` tool), use
/// [`html_to_markdown`] after making the request independently.
pub async fn fetch_url_to_markdown(url: &str) -> Result<String> {
    let response = reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;

    let status = response.status();
    if status.is_server_error() {
        anyhow::bail!("Server error: HTTP {}", status);
    }
    if status.is_client_error() {
        anyhow::bail!("Client error: HTTP {}", status);
    }

    let html_content = response.text().await?;
    html_to_markdown(&html_content)
}

/// Convert HTML content to markdown, skipping script, style, footer,
/// img, and svg tags.
pub fn html_to_markdown(html: &str) -> Result<String> {
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "footer", "img", "svg"])
        .build();
    Ok(converter.convert(html)?)
}
