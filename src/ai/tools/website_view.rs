use std::time::Duration;

use crate::openai::{Function, Parameters, Property, RecoverableToolError, ToolCall, ToolType, parse_tool_args};
use anyhow::{Context, Error, Result};
use async_trait::async_trait;
use htmd::HtmlToMarkdown;
use reqwest;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct WebsiteViewProps {
    pub url: Property,
}

#[derive(Deserialize)]
pub struct WebsiteViewArgs {
    pub url: String,
}

#[derive(Serialize)]
pub struct WebsiteViewTool {
    pub r#type: ToolType,
    pub function: Function<WebsiteViewProps>,
}

#[async_trait]
impl ToolCall for WebsiteViewTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: WebsiteViewArgs = parse_tool_args(args)?;
        // let url = fn_args.url;

        // Clean the URL, stripping away unnecessary URL params like
        // UTM codes. This breaks sites that rely on query params for
        // viewing the content but that's a fair tradeoff to prevent
        // accidental data leakage.
        let url = reqwest::Url::parse(fn_args.url.trim())
            .context(fn_args.url)
            .expect("Invalid URL");
        let host = url.host_str().expect("Missing host");
        let port = url
            .port()
            .map(|p| format!(":{}", p))
            .unwrap_or_default();
        let clean_url = format!("{}://{}{}{}", url.scheme(), host, port, url.path());

        // TODO: Rewrite URLs based on rules. For example, use mirrors
        // or archives for certain sites.

        // TODO: Validate the URL is acceptable to view given the AI
        // agent's context. This partially mitigates prompt injection
        // attacks by constraining the set of possible websites that
        // can be requested.
        // Does this matter if we only allow GET requests and no
        // params?

        // Fetch the HTML content from the URL
        let response = reqwest::Client::new()
            .get(&clean_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await;

        // Handle request errors like timeouts
        let content = match response {
            Ok(resp) => {
                // Check for HTTP-level errors before processing the
                // body. reqwest does not treat non-2xx as errors by
                // default, so we need to check explicitly.
                let status = resp.status();
                if status.is_server_error() {
                    tracing::warn!("Website view failed with HTTP {}.", status);
                    return Err(RecoverableToolError::new(
                        &format!(
                            "Website view failed with HTTP {} (server error). The server may be temporarily unavailable — try again.",
                            status,
                        ),
                    )
                    .into());
                }
                if status.is_client_error() {
                    tracing::warn!("Website view failed with HTTP {}.", status);
                    return Ok(format!(
                        "Website view failed with HTTP status code {}",
                        status,
                    ));
                }

                // Convert HTML to markdown
                let html_content = resp.text().await?;
                let converter = HtmlToMarkdown::builder()
                    .skip_tags(vec!["script", "style", "footer", "img", "svg"])
                    .build();
                converter.convert(&html_content)?
            }
            Err(e) => {
                // If the request failed, provide a default answer so we
                // don't crash the whole chat. For example: "Fetching the link
                // failed and due to a 500 status code"
                match e {
                    i if i.is_timeout() => {
                        tracing::warn!("Website view failed due to timeout.");
                        return Err(RecoverableToolError::new(
                            "Request timed out. The server may be slow or unavailable — try again or use a different source.",
                        )
                        .into());
                    }
                    i if i.is_request() => {
                        tracing::warn!("Website view failed due to request sending error.");
                        String::from("Request was not able to be sent. Do not retry.")
                    }
                    i if i.is_status() => {
                        // HTTP status errors are handled in the Ok(resp)
                        // branch above. This path is a safety net in case
                        // reqwest is configured to treat non-2xx as errors.
                        let msg = i
                            .status()
                            .map(|s| format!("Website view failed with HTTP status code {}", s))
                            .unwrap_or_else(|| format!("Website view failed: {}", i));
                        tracing::warn!("{}", msg);
                        String::from(msg)
                    }
                    _ => anyhow::bail!("Website view failed: {}", e),
                }
            }
        };

        // TODO: Limit the amount of content returned to avoid filling
        // the context window with noise.
        Ok(content)
    }

    fn function_name(&self) -> String {
        self.function.name.clone()
    }
}

impl WebsiteViewTool {
    pub fn new() -> Self {
        let function = Function {
            name: String::from("view_website"),
            description: String::from(
                "Fetch and convert a website's content to markdown for viewing.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: WebsiteViewProps {
                    url: Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "The URL of the website to fetch and convert to markdown.",
                        ),
                        r#enum: None,
                    },
                },
                required: vec![String::from("url")],
                additional_properties: false,
            },
            strict: true,
        };
        Self {
            r#type: ToolType::Function,
            function,
        }
    }
}

impl Default for WebsiteViewTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn it_returns_ok_for_successful_response() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(200)
            .with_header("content-type", "text/html")
            .with_body("<html><body><h1>Hello World</h1></body></html>")
            .create();

        let tool = WebsiteViewTool::new();
        let url = format!(r#"{{"url": "{}/"}}"#, server.url());
        let result = tool.call(&url).await;

        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("Hello World"));
    }

    #[tokio::test]
    async fn it_returns_recoverable_error_for_server_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(500)
            .with_header("content-type", "text/html")
            .with_body("Internal Server Error")
            .create();

        let tool = WebsiteViewTool::new();
        let url = format!(r#"{{"url": "{}/"}}"#, server.url());
        let result = tool.call(&url).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
        assert!(recoverable
            .unwrap()
            .message
            .contains("server error"));
    }

    #[tokio::test]
    async fn it_returns_ok_string_for_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(404)
            .with_header("content-type", "text/html")
            .with_body("Not Found")
            .create();

        let tool = WebsiteViewTool::new();
        let url = format!(r#"{{"url": "{}/"}}"#, server.url());
        let result = tool.call(&url).await;

        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("HTTP status code 404"));
    }

    #[tokio::test]
    async fn it_returns_recoverable_error_for_service_unavailable() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/")
            .with_status(503)
            .with_header("content-type", "text/html")
            .with_body("Service Unavailable")
            .create();

        let tool = WebsiteViewTool::new();
        let url = format!(r#"{{"url": "{}/"}}"#, server.url());
        let result = tool.call(&url).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        let recoverable = err.downcast_ref::<RecoverableToolError>();
        assert!(recoverable.is_some());
        assert!(recoverable
            .unwrap()
            .message
            .contains("server error"));
    }
}
