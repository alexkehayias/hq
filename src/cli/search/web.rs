use crate::ai::tools::website_view::{fetch_url_response, html_to_markdown};
use anyhow::Result;

pub async fn run(url: String) -> Result<()> {
    let response = fetch_url_response(&url).await?;

    let status = response.status();
    if status.is_server_error() {
        anyhow::bail!(
            "Website view failed with HTTP {} (server error).",
            status,
        );
    }
    if status.is_client_error() {
        anyhow::bail!("Website view failed with HTTP status code {}", status);
    }

    let html_content = response.text().await?;
    let content = html_to_markdown(&html_content)?;

    println!("{}", content);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn it_fetches_and_converts_successfully() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Hello World</h1></body></html>")
            .create();

        let result = run(format!("{}/", server.url())).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn it_returns_error_for_server_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(500)
            .with_body("Internal Server Error")
            .create();

        let result = run(format!("{}/", server.url())).await;

        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("server error"));
    }

    #[tokio::test]
    async fn it_returns_error_for_client_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(404)
            .with_body("Not Found")
            .create();

        let result = run(format!("{}/", server.url())).await;

        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("HTTP status code 404"));
    }
}
