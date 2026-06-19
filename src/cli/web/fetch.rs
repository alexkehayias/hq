use anyhow::{Context, Result};
use std::time::Duration;

/// Fetch a URL and print its content as markdown to stdout.
pub async fn run(url: String) -> Result<()> {
    let parsed_url = reqwest::Url::parse(&url)
        .context(format!("Invalid URL: {}", url))?;
    let host = parsed_url.host_str().context("Missing host")?;
    let port = parsed_url
        .port()
        .map(|p| format!(":{}", p))
        .unwrap_or_default();
    let clean_url = format!("{}://{}{}{}", parsed_url.scheme(), host, port, parsed_url.path());

    let response = reqwest::Client::new()
        .get(&clean_url)
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
    let converter = htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "footer", "img", "svg"])
        .build();
    let markdown = converter.convert(&html_content)?;

    println!("{}", markdown);
    Ok(())
}
