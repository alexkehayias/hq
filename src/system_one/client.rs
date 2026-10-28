//! HTTP client for the System One API.
//!
//! The base URL, API key, and model are all supplied by the caller, so the
//! client works against a hosted service or any locally hosted inference server
//! that implements the same endpoints.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::header::CONTENT_TYPE;

use crate::core::AppConfig;

use super::models::{
    ApiErrorBody, Content, ModelList, ModelMetadata, Question, SystemOneRequest, SystemOneResponse,
};

/// Header the API requires when a request carries images or box questions.
const DANGER_ZONE_IMAGES: &str = "TypeSafe-Danger-Zone-Images";

/// Timeout for a single System One request. These calls are fast, unlike
/// completions.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// A client for the System One API.
#[derive(Clone)]
pub struct SystemOneClient {
    base_url: String,
    api_key: Option<String>,
    model: String,
    http: reqwest::Client,
}

impl SystemOneClient {
    /// Build a client for `base_url` (for example `http://localhost:8000`).
    ///
    /// `api_key` is optional: a locally hosted server often accepts
    /// unauthenticated requests.
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key,
            model: model.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Build a client from the app config, reusing the same host and API key as
    /// the completions API so a single local server can serve both.
    pub fn from_config(config: &AppConfig) -> Self {
        Self::new(
            config.openai_api_hostname.clone(),
            Some(config.openai_api_key.clone()),
            config.system_one_model.clone(),
        )
    }

    /// Replace the model used by [`SystemOneClient::system_one`].
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// The model used by [`SystemOneClient::system_one`].
    pub fn model(&self) -> &str {
        &self.model
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut builder = self
            .http
            .request(method, self.url(path))
            .timeout(REQUEST_TIMEOUT);
        if let Some(key) = &self.api_key {
            builder = builder.bearer_auth(key);
        }
        builder
    }

    /// Ask one or more questions about `state`, using the client's model.
    pub async fn system_one(
        &self,
        state: impl Into<Content>,
        questions: BTreeMap<String, Question>,
    ) -> Result<SystemOneResponse> {
        let mut request = SystemOneRequest::new(self.model.clone(), state);
        request.questions = questions;
        self.ask(request).await
    }

    /// Send a fully-built request to `POST /v1/systemone`.
    pub async fn ask(&self, request: SystemOneRequest) -> Result<SystemOneResponse> {
        let mut builder = self
            .request(reqwest::Method::POST, "/v1/systemone")
            .header(CONTENT_TYPE, "application/json")
            .json(&request);
        if request.has_images() {
            builder = builder.header(DANGER_ZONE_IMAGES, "true");
        }

        let response = builder.send().await.context("System One request failed")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("reading System One response failed")?;

        if !status.is_success() {
            bail!("System One API returned {status}: {}", error_message(&body));
        }

        serde_json::from_str(&body)
            .with_context(|| format!("parsing System One response failed: {body}"))
    }

    /// List the models and aliases available to the account.
    pub async fn list_models(&self) -> Result<Vec<ModelMetadata>> {
        let response = self
            .request(reqwest::Method::GET, "/v1/models")
            .send()
            .await
            .context("System One list-models request failed")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("reading System One response failed")?;

        if !status.is_success() {
            bail!("System One API returned {status}: {}", error_message(&body));
        }

        let list: ModelList = serde_json::from_str(&body)
            .with_context(|| format!("parsing model list failed: {body}"))?;
        Ok(list.models)
    }
}

/// Extract a human-readable message from an error response body, falling back
/// to the raw body when it isn't the expected JSON shape.
fn error_message(body: &str) -> String {
    match serde_json::from_str::<ApiErrorBody>(body) {
        Ok(error) => error.message().unwrap_or_else(|| body.to_string()),
        Err(_) => body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_one::models::{Asset, ImageFormat};

    #[tokio::test]
    async fn ask_posts_request_and_parses_answers() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/systemone")
            .match_header("authorization", "Bearer test-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "model": "jev-latest",
                    "answers": {"billing": {"type": "noul", "noul": 0.98}},
                    "usage": {"input_tokens": 120, "output_tokens": 12}
                }"#,
            )
            .create();

        let client = SystemOneClient::new(server.url(), Some("test-key".to_string()), "jev-latest");
        let questions = BTreeMap::from([(
            "billing".to_string(),
            Question::noul("Is this message about billing?"),
        )]);
        let response = client
            .system_one("I was charged twice.", questions)
            .await
            .unwrap();

        mock.assert();
        assert_eq!(response.noul("billing"), Some(0.98));
        assert_eq!(response.usage.input_tokens, 120);
    }

    #[tokio::test]
    async fn sets_images_header_when_request_has_assets() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/systemone")
            .match_header("TypeSafe-Danger-Zone-Images", "true")
            .with_status(200)
            .with_body(
                r#"{
                    "model": "m",
                    "answers": {"b": {"type": "box", "box": {"top": 0.1, "bottom": 0.2, "left": 0.3, "right": 0.4}, "confidence": null}},
                    "usage": {"input_tokens": 1, "output_tokens": 1}
                }"#,
            )
            .create();

        let client = SystemOneClient::new(server.url(), None, "m");
        let request = SystemOneRequest::new("m", "photo")
            .question("b", Question::boxed("Locate the rectangle."))
            .asset("photo", Asset::from_bytes(ImageFormat::Png, &[0, 1, 2]));
        let response = client.ask(request).await.unwrap();

        mock.assert();
        assert_eq!(
            response
                .answer("b")
                .unwrap()
                .box_coordinates()
                .unwrap()
                .left,
            0.3
        );
    }

    #[tokio::test]
    async fn list_models_returns_models() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("GET", "/v1/models")
            .with_status(200)
            .with_body(
                r#"{"models":[{"name":"jev-latest","description":"General-purpose.","release_date":"2026-09-15"}]}"#,
            )
            .create();

        let client = SystemOneClient::new(server.url(), None, "jev-latest");
        let models = client.list_models().await.unwrap();

        mock.assert();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "jev-latest");
    }

    #[tokio::test]
    async fn error_body_surfaces_detail() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/systemone")
            .with_status(422)
            .with_body(
                r#"{"detail":[{"loc":["body","state"],"msg":"Field required","type":"missing"}]}"#,
            )
            .create();

        let client = SystemOneClient::new(server.url(), None, "m");
        let error = client
            .system_one("state", BTreeMap::new())
            .await
            .unwrap_err();

        mock.assert();
        let message = error.to_string();
        assert!(message.contains("422"), "{message}");
        assert!(message.contains("Field required"), "{message}");
    }
}
