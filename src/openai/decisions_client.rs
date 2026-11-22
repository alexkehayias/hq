//! Configured client for the [OpenAI Decisions API] backed by a local
//! inference server.
//!
//! The free functions in [`super::decisions`] take the host, API key, and
//! model on every call and build a fresh HTTP client each time. This module
//! binds those settings — plus the connection pool and timeout — once, which
//! is what an integration with a long-running local inference server wants.
//!
//! Use [`DecisionsClient::from_config`] to follow the server's local
//! inference settings (`HQ_LOCAL_LLM_HOST`, `HQ_LOCAL_LLM_MODEL`, and
//! `OPENAI_API_KEY`), or [`DecisionsClient::new`] to target a server that
//! does not require authentication.
//!
//! [OpenAI Decisions API]: https://developers.openai.com/api/reference/resources/decisions/methods/create

use std::time::Duration;

use anyhow::{Error, Result};

use crate::core::AppConfig;

use super::decisions::{
    DEFAULT_TIMEOUT, Decision, DecisionInput, DecisionOptions, Question, http_client, send_decision,
};

/// A reusable client for `POST /v1/decisions`.
///
/// The client is cheap to clone: clones share the underlying HTTP connection
/// pool and configuration.
#[derive(Clone, Debug)]
pub struct DecisionsClient {
    http: reqwest::Client,
    api_hostname: String,
    api_key: Option<String>,
    model: String,
}

impl DecisionsClient {
    /// Build a client for `api_hostname` and `model` with no authentication.
    ///
    /// `api_hostname` may include the scheme and a trailing slash;
    /// `/v1/decisions` is appended. Local inference servers commonly run
    /// without authentication, so no `Authorization` header is sent until
    /// [`with_api_key`](Self::with_api_key) is called.
    pub fn new(api_hostname: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: http_client(DEFAULT_TIMEOUT),
            api_hostname: api_hostname.into(),
            api_key: None,
            model: model.into(),
        }
    }

    /// Build a client from the app's local inference server configuration.
    ///
    /// The host and model come from `HQ_LOCAL_LLM_HOST` and
    /// `HQ_LOCAL_LLM_MODEL`, and the bearer token from `OPENAI_API_KEY`,
    /// matching how the chat completions client is configured.
    pub fn from_config(config: &AppConfig) -> Self {
        Self::new(
            config.openai_api_hostname.clone(),
            config.openai_model.clone(),
        )
        .with_api_key(config.openai_api_key.clone())
    }

    /// Send `Authorization: Bearer <api_key>` with every request.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Replace the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.http = http_client(timeout);
        self
    }

    /// Ask `questions` about `input`.
    pub async fn decision(
        &self,
        input: &DecisionInput,
        questions: &[Question],
    ) -> Result<Decision, Error> {
        self.decision_with_options(&DecisionOptions::default(), input, questions)
            .await
    }

    /// Like [`decision`](Self::decision), with prompt-caching and
    /// safety-identifier options.
    pub async fn decision_with_options(
        &self,
        options: &DecisionOptions,
        input: &DecisionInput,
        questions: &[Question],
    ) -> Result<Decision, Error> {
        send_decision(
            &self.http,
            &self.api_hostname,
            self.api_key.as_deref(),
            &self.model,
            options,
            input,
            questions,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AppConfig;
    use crate::openai::{
        Answer, ChoiceOption, ChoiceProbability, ChoiceValue, Instructions, PromptCacheMode,
        PromptCacheOptions, PromptCacheTtl,
    };
    use mockito::{Matcher, Server};
    use serde_json::json;

    fn test_config(api_hostname: String) -> AppConfig {
        AppConfig {
            notes_path: "notes".to_string(),
            index_path: "index".to_string(),
            vec_db_path: "db".to_string(),
            storage_path: "storage".to_string(),
            skills_path: "skills".to_string(),
            deploy_key_path: "deploy-key".to_string(),
            vapid_key_path: "vapid-key".to_string(),
            note_search_api_url: "http://localhost".to_string(),
            gmail_api_client_id: "gmail-id".to_string(),
            gmail_api_client_secret: "gmail-secret".to_string(),
            google_search_api_key: "google-key".to_string(),
            google_search_cx_id: "google-cx".to_string(),
            openai_model: "local-model".to_string(),
            openai_api_hostname: api_hostname,
            openai_api_key: "local-key".to_string(),
            system_message: "test".to_string(),
        }
    }

    fn predicate_question() -> Question {
        Question::Predicate {
            instructions: Instructions::from("Does the customer report a damaged item?"),
            name: Some("damaged".to_string()),
        }
    }

    #[tokio::test]
    async fn new_client_omits_authorization() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .match_header("authorization", Matcher::Missing)
            .match_body(Matcher::PartialJson(json!({
                "model": "local-model",
                "input": "The package arrived with a broken screen.",
                "questions": [{
                    "type": "predicate",
                    "name": "damaged",
                    "instructions": "Does the customer report a damaged item?"
                }]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"local-model","answers":[{"type":"predicate","name":"damaged","probability":0.95}]}"#)
            .create();

        let client = DecisionsClient::new(server.url(), "local-model");
        let decision = client
            .decision(
                &DecisionInput::from("The package arrived with a broken screen."),
                &[predicate_question()],
            )
            .await
            .unwrap();

        mock.assert();
        assert_eq!(
            decision.answers[0],
            Answer::Predicate {
                name: Some("damaged".to_string()),
                probability: 0.95
            }
        );
    }

    #[tokio::test]
    async fn with_api_key_sends_bearer_token() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .match_header("authorization", "Bearer local-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"local-model","answers":[]}"#)
            .create();

        let client = DecisionsClient::new(server.url(), "local-model").with_api_key("local-key");
        client
            .decision(&DecisionInput::from("evidence"), &[predicate_question()])
            .await
            .unwrap();

        mock.assert();
    }

    #[tokio::test]
    async fn from_config_uses_local_inference_settings() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .match_header("authorization", "Bearer local-key")
            .match_body(Matcher::PartialJson(json!({
                "model": "local-model",
                "input": "evidence",
                "questions": [{
                    "type": "predicate",
                    "instructions": "Does the customer report a damaged item?"
                }]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"local-model","answers":[]}"#)
            .create();

        let client = DecisionsClient::from_config(&test_config(server.url()));
        client
            .decision(&DecisionInput::from("evidence"), &[predicate_question()])
            .await
            .unwrap();

        mock.assert();
    }

    #[tokio::test]
    async fn decision_with_options_forwards_request_options() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .match_body(Matcher::PartialJson(json!({
                "prompt_cache_key": "tenant-1",
                "prompt_cache_options": {"mode": "explicit", "ttl": "30m"},
                "safety_identifier": "user-1"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"local-model","answers":[]}"#)
            .create();

        let options = DecisionOptions {
            prompt_cache_key: Some("tenant-1".to_string()),
            prompt_cache_options: Some(PromptCacheOptions {
                mode: Some(PromptCacheMode::Explicit),
                ttl: Some(PromptCacheTtl::ThirtyMinutes),
            }),
            safety_identifier: Some("user-1".to_string()),
        };
        let client = DecisionsClient::new(server.url(), "local-model");

        client
            .decision_with_options(&options, &DecisionInput::from("evidence"), &[])
            .await
            .unwrap();

        mock.assert();
    }

    #[tokio::test]
    async fn deserializes_typed_choice_answer() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "model": "local-model",
                    "answers": [{
                        "type": "choice",
                        "name": "department",
                        "choice": "billing",
                        "confidence": 0.8,
                        "probabilities": [
                            {"value": "billing", "probability": 0.8},
                            {"value": "support", "probability": 0.2}
                        ]
                    }]
                }"#,
            )
            .create();

        let client = DecisionsClient::new(server.url(), "local-model");
        let decision = client
            .decision(
                &DecisionInput::from("Please refund the duplicate charge."),
                &[Question::Choice {
                    instructions: Instructions::from("Which department should handle this?"),
                    choices: vec![
                        ChoiceOption {
                            value: ChoiceValue::from("billing"),
                            description: None,
                        },
                        ChoiceOption {
                            value: ChoiceValue::from("support"),
                            description: None,
                        },
                    ],
                    name: Some("department".to_string()),
                }],
            )
            .await
            .unwrap();

        mock.assert();
        assert_eq!(
            decision.answers[0],
            Answer::Choice {
                name: Some("department".to_string()),
                choice: ChoiceValue::String("billing".to_string()),
                confidence: 0.8,
                probabilities: vec![
                    ChoiceProbability {
                        value: ChoiceValue::String("billing".to_string()),
                        probability: 0.8,
                    },
                    ChoiceProbability {
                        value: ChoiceValue::String("support".to_string()),
                        probability: 0.2,
                    },
                ],
            }
        );
    }

    #[tokio::test]
    async fn surfaces_error_status() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(503)
            .with_body("model is loading")
            .create();

        let client = DecisionsClient::new(server.url(), "local-model");
        let error = client
            .decision(&DecisionInput::from("evidence"), &[predicate_question()])
            .await
            .unwrap_err()
            .to_string();

        mock.assert();
        assert!(error.contains("503"));
        assert!(error.contains("model is loading"));
    }
}
