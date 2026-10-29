//! Client for OpenAI's [Decisions API](https://developers.openai.com/api/docs/guides/decisions).
//!
//! The Decisions API evaluates shared evidence (`input`) against one or more
//! `questions` and returns typed `answers`: a probability for a `predicate`,
//! a value from a fixed set for a `choice`, or a score against ordered levels.
//!
//! Requests are sent to `POST {api_hostname}/v1/decisions`. Like the chat
//! completions client in [`crate::openai::core`], the host, API key, and model
//! are caller-supplied so this can target a local OpenAI-compatible inference
//! server as well as `api.openai.com`.
//!
//! This module is intentionally standalone: it is not wired into `AppConfig`
//! or the chat loop yet.

use std::time::Duration;

use anyhow::{Error, Result};
use serde::{Deserialize, Serialize};

use super::core::{ContentPart, FunctionCallFn, Message};

/// The model used by OpenAI's hosted Decisions API.
///
/// Local servers may expose a different model name; callers pass the model
/// explicitly to [`decision`].
pub const DEFAULT_MODEL: &str = "gpt-6-luna";

/// Shared evidence evaluated by every question.
///
/// Either a plain text string or an ordered array of conversation items.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum DecisionInput {
    Text(String),
    Items(Vec<InputItem>),
}

impl From<&str> for DecisionInput {
    fn from(text: &str) -> Self {
        Self::Text(text.to_string())
    }
}

impl From<String> for DecisionInput {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<Vec<InputItem>> for DecisionInput {
    fn from(items: Vec<InputItem>) -> Self {
        Self::Items(items)
    }
}

/// An item in a conversation used as `input` evidence.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type")]
pub enum InputItem {
    #[serde(rename = "message")]
    Message(Message),
    #[serde(rename = "function_call")]
    FunctionCall(FunctionCallItem),
    #[serde(rename = "function_call_output")]
    FunctionCallOutput(FunctionCallOutput),
    #[serde(rename = "custom_tool_call")]
    CustomToolCall(CustomToolCall),
    #[serde(rename = "custom_tool_call_output")]
    CustomToolCallOutput(CustomToolCallOutput),
}

/// The status of a tool call or tool output item.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    InProgress,
    Completed,
    Incomplete,
}

/// A function call in conversation evidence.
///
/// The Decisions API puts `name` and `arguments` directly on the item, which
/// is what [`FunctionCallFn`] carries, so it is flattened into this struct.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct FunctionCallItem {
    pub call_id: String,
    #[serde(flatten)]
    pub function: FunctionCallFn,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ItemStatus>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct FunctionCallOutput {
    pub call_id: String,
    pub output: ToolOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ItemStatus>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct CustomToolCall {
    pub call_id: String,
    pub name: String,
    pub input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ItemStatus>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct CustomToolCallOutput {
    pub call_id: String,
    pub output: ToolOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ItemStatus>,
}

/// Text or images returned by a tool.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum ToolOutput {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl From<&str> for ToolOutput {
    fn from(text: &str) -> Self {
        Self::Text(text.to_string())
    }
}

impl From<String> for ToolOutput {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

/// The instructions for a question.
///
/// Either plain text or an ordered array of content parts.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum Instructions {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl From<&str> for Instructions {
    fn from(text: &str) -> Self {
        Self::Text(text.to_string())
    }
}

impl From<String> for Instructions {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<Vec<ContentPart>> for Instructions {
    fn from(parts: Vec<ContentPart>) -> Self {
        Self::Parts(parts)
    }
}

/// A question to ask about the input.
///
/// Answers are returned in the same order as the questions, and each answer
/// echoes the question's optional `name`.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type")]
pub enum Question {
    /// Estimate how likely it is that a statement about the input is true.
    #[serde(rename = "predicate")]
    Predicate {
        instructions: Instructions,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Choose from the supplied options based on the input.
    #[serde(rename = "choice")]
    Choice {
        instructions: Instructions,
        choices: Vec<ChoiceOption>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// Rate the input against the supplied ordered levels.
    #[serde(rename = "score")]
    Score {
        instructions: Instructions,
        levels: Vec<ScoreLevel>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ChoiceOption {
    pub value: ChoiceValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A typed choice value. A string and a boolean with the same text are
/// distinct.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum ChoiceValue {
    String(String),
    Bool(bool),
}

impl From<&str> for ChoiceValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<String> for ChoiceValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<bool> for ChoiceValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ScoreLevel {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DecisionOptions {
    pub prompt_cache_key: Option<String>,
    pub prompt_cache_options: Option<PromptCacheOptions>,
    pub safety_identifier: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PromptCacheOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<PromptCacheMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<PromptCacheTtl>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheMode {
    Implicit,
    Explicit,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum PromptCacheTtl {
    #[serde(rename = "30m")]
    ThirtyMinutes,
}

#[derive(Serialize, Debug)]
pub struct DecisionRequest<'a> {
    pub model: &'a str,
    pub input: &'a DecisionInput,
    pub questions: &'a [Question],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_options: Option<&'a PromptCacheOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<&'a str>,
}

/// Ask questions about `input` and return typed answers.
///
/// `api_hostname` should include the scheme (for example
/// `https://api.openai.com` or `http://localhost:8000`); `/v1/decisions` is
/// appended. `model` is caller-supplied so a local server can use its own
/// model name.
pub async fn decision(
    api_hostname: &str,
    api_key: &str,
    model: &str,
    input: &DecisionInput,
    questions: &[Question],
) -> Result<Decision, Error> {
    decision_with_options(
        api_hostname,
        api_key,
        model,
        &DecisionOptions::default(),
        input,
        questions,
    )
    .await
}

/// Like [`decision`], with prompt-caching and safety-identifier options.
pub async fn decision_with_options(
    api_hostname: &str,
    api_key: &str,
    model: &str,
    options: &DecisionOptions,
    input: &DecisionInput,
    questions: &[Question],
) -> Result<Decision, Error> {
    let payload = DecisionRequest {
        model,
        input,
        questions,
        prompt_cache_key: options.prompt_cache_key.as_deref(),
        prompt_cache_options: options.prompt_cache_options.as_ref(),
        safety_identifier: options.safety_identifier.as_deref(),
    };
    let url = format!("{}/v1/decisions", api_hostname.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(60 * 10))
        .json(&payload)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Decisions API returned error status {status}: {body}");
    }

    Ok(response.json().await?)
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Decision {
    pub answers: Vec<Answer>,
    pub model: String,
    #[serde(default)]
    pub usage: Usage,
}

/// A typed answer. A question may return a refusal instead of an answer;
/// other questions in the same request still receive answers.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "type")]
pub enum Answer {
    #[serde(rename = "predicate")]
    Predicate {
        name: Option<String>,
        probability: f64,
    },
    #[serde(rename = "choice")]
    Choice {
        name: Option<String>,
        choice: ChoiceValue,
        confidence: f64,
        probabilities: Vec<ChoiceProbability>,
    },
    #[serde(rename = "score")]
    Score {
        name: Option<String>,
        score: f64,
        confidence: f64,
        probabilities: Vec<ScoreProbability>,
    },
    #[serde(rename = "refusal")]
    Refusal { name: Option<String> },
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ChoiceProbability {
    pub value: ChoiceValue,
    pub probability: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ScoreProbability {
    pub label: String,
    pub value: i64,
    pub probability: f64,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub input_tokens_details: InputTokensDetails,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub output_tokens_details: OutputTokensDetails,
    #[serde(default)]
    pub total_tokens: i64,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct InputTokensDetails {
    #[serde(default)]
    pub cached_tokens: i64,
    #[serde(default)]
    pub cache_write_tokens: i64,
}

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct OutputTokensDetails {
    #[serde(default)]
    pub reasoning_tokens: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ContentPart, FunctionCallFn, Role};
    use serde_json::json;

    #[test]
    fn serializes_text_input_and_predicate_question() {
        let input = DecisionInput::from("The package arrived with a broken screen.");
        let questions = vec![Question::Predicate {
            instructions: Instructions::from("Does the customer report a damaged item?"),
            name: Some("damaged".to_string()),
        }];

        let payload = serde_json::to_value(DecisionRequest {
            model: DEFAULT_MODEL,
            input: &input,
            questions: &questions,
            prompt_cache_key: None,
            prompt_cache_options: None,
            safety_identifier: None,
        })
        .unwrap();

        assert_eq!(
            payload,
            json!({
                "model": "gpt-6-luna",
                "input": "The package arrived with a broken screen.",
                "questions": [{
                    "type": "predicate",
                    "instructions": "Does the customer report a damaged item?",
                    "name": "damaged"
                }]
            })
        );
    }

    #[test]
    fn serializes_message_input_with_parts() {
        let input = DecisionInput::Items(vec![InputItem::Message(Message::new_with_parts(
            Role::User,
            vec![
                ContentPart::input_text("Evaluate the ticket using the order evidence."),
                ContentPart::input_json(json!({
                    "ticket": { "body": "Please refund the duplicate charge." },
                    "order": { "charges": [49, 49] }
                })),
            ],
        ))]);

        let payload = serde_json::to_value(&input).unwrap();

        assert_eq!(
            payload,
            json!([{
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "Evaluate the ticket using the order evidence." },
                    {
                        "type": "input_json",
                        "data": {
                            "ticket": { "body": "Please refund the duplicate charge." },
                            "order": { "charges": [49, 49] }
                        }
                    }
                ]
            }])
        );
    }

    #[test]
    fn serializes_conversation_with_tool_calls() {
        let input = DecisionInput::Items(vec![
            InputItem::Message(Message::new(Role::User, "What is the order status?")),
            InputItem::FunctionCall(FunctionCallItem {
                call_id: "call_1".to_string(),
                function: FunctionCallFn {
                    arguments: r#"{"id":42}"#.to_string(),
                    name: "lookup_order".to_string(),
                },
                id: None,
                namespace: None,
                status: Some(ItemStatus::Completed),
            }),
            InputItem::FunctionCallOutput(FunctionCallOutput {
                call_id: "call_1".to_string(),
                output: ToolOutput::from("shipped"),
                id: None,
                status: Some(ItemStatus::Completed),
            }),
        ]);

        let payload = serde_json::to_value(&input).unwrap();

        assert_eq!(
            payload,
            json!([
                { "type": "message", "role": "user", "content": "What is the order status?" },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "lookup_order",
                    "arguments": "{\"id\":42}",
                    "status": "completed"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": "shipped",
                    "status": "completed"
                }
            ])
        );
    }

    #[test]
    fn serializes_choice_and_score_questions() {
        let questions = vec![
            Question::Choice {
                instructions: Instructions::from("Which department should handle this?"),
                choices: vec![
                    ChoiceOption {
                        value: ChoiceValue::from("billing"),
                        description: Some("Billing and invoices".to_string()),
                    },
                    ChoiceOption {
                        value: ChoiceValue::from(true),
                        description: None,
                    },
                ],
                name: Some("department".to_string()),
            },
            Question::Score {
                instructions: Instructions::from("How severe is the issue?"),
                levels: vec![
                    ScoreLevel {
                        label: "low".to_string(),
                        description: None,
                    },
                    ScoreLevel {
                        label: "high".to_string(),
                        description: Some("Needs immediate attention".to_string()),
                    },
                ],
                name: Some("severity".to_string()),
            },
        ];

        let payload = serde_json::to_value(&questions).unwrap();

        assert_eq!(
            payload,
            json!([
                {
                    "type": "choice",
                    "instructions": "Which department should handle this?",
                    "choices": [
                        { "value": "billing", "description": "Billing and invoices" },
                        { "value": true }
                    ],
                    "name": "department"
                },
                {
                    "type": "score",
                    "instructions": "How severe is the issue?",
                    "levels": [
                        { "label": "low" },
                        { "label": "high", "description": "Needs immediate attention" }
                    ],
                    "name": "severity"
                }
            ])
        );
    }

    #[test]
    fn serializes_prompt_cache_options() {
        let input = DecisionInput::from("evidence");
        let questions = vec![Question::Predicate {
            instructions: Instructions::from("Is it true?"),
            name: None,
        }];
        let options = DecisionOptions {
            prompt_cache_key: Some("tenant-1".to_string()),
            prompt_cache_options: Some(PromptCacheOptions {
                mode: Some(PromptCacheMode::Explicit),
                ttl: Some(PromptCacheTtl::ThirtyMinutes),
            }),
            safety_identifier: Some("user-1".to_string()),
        };

        let payload = serde_json::to_value(DecisionRequest {
            model: DEFAULT_MODEL,
            input: &input,
            questions: &questions,
            prompt_cache_key: options.prompt_cache_key.as_deref(),
            prompt_cache_options: options.prompt_cache_options.as_ref(),
            safety_identifier: options.safety_identifier.as_deref(),
        })
        .unwrap();

        assert_eq!(
            payload,
            json!({
                "model": "gpt-6-luna",
                "input": "evidence",
                "questions": [{ "type": "predicate", "instructions": "Is it true?" }],
                "prompt_cache_key": "tenant-1",
                "prompt_cache_options": { "mode": "explicit", "ttl": "30m" },
                "safety_identifier": "user-1"
            })
        );
    }

    #[test]
    fn deserializes_all_answer_types() {
        let body = json!({
            "model": "gpt-6-luna",
            "answers": [
                { "type": "predicate", "name": "damaged", "probability": 0.95 },
                {
                    "type": "choice",
                    "name": "department",
                    "choice": "billing",
                    "confidence": 0.8,
                    "probabilities": [
                        { "value": "billing", "probability": 0.8 },
                        { "value": "support", "probability": 0.2 }
                    ]
                },
                {
                    "type": "score",
                    "name": "severity",
                    "score": 1.5,
                    "confidence": 0.7,
                    "probabilities": [
                        { "label": "low", "value": 0, "probability": 0.5 },
                        { "label": "high", "value": 1, "probability": 0.5 }
                    ]
                },
                { "type": "refusal", "name": null }
            ],
            "usage": {
                "input_tokens": 42,
                "input_tokens_details": { "cached_tokens": 10, "cache_write_tokens": 0 },
                "output_tokens": 0,
                "output_tokens_details": { "reasoning_tokens": 0 },
                "total_tokens": 42
            }
        });

        let decision: Decision = serde_json::from_value(body).unwrap();

        assert_eq!(decision.answers.len(), 4);
        assert_eq!(
            decision.answers[0],
            Answer::Predicate {
                name: Some("damaged".to_string()),
                probability: 0.95
            }
        );
        assert_eq!(
            decision.answers[1],
            Answer::Choice {
                name: Some("department".to_string()),
                choice: ChoiceValue::String("billing".to_string()),
                confidence: 0.8,
                probabilities: vec![
                    ChoiceProbability {
                        value: ChoiceValue::String("billing".to_string()),
                        probability: 0.8
                    },
                    ChoiceProbability {
                        value: ChoiceValue::String("support".to_string()),
                        probability: 0.2
                    }
                ]
            }
        );
        assert_eq!(
            decision.answers[2],
            Answer::Score {
                name: Some("severity".to_string()),
                score: 1.5,
                confidence: 0.7,
                probabilities: vec![
                    ScoreProbability {
                        label: "low".to_string(),
                        value: 0,
                        probability: 0.5
                    },
                    ScoreProbability {
                        label: "high".to_string(),
                        value: 1,
                        probability: 0.5
                    }
                ]
            }
        );
        assert_eq!(decision.answers[3], Answer::Refusal { name: None });
        assert_eq!(decision.usage.input_tokens, 42);
        assert_eq!(decision.usage.input_tokens_details.cached_tokens, 10);
        assert_eq!(decision.usage.total_tokens, 42);
    }

    #[test]
    fn deserializes_boolean_choice() {
        let answer: Answer = serde_json::from_value(json!({
            "type": "choice",
            "name": "approved",
            "choice": true,
            "confidence": 0.99,
            "probabilities": [
                { "value": true, "probability": 0.99 },
                { "value": false, "probability": 0.01 }
            ]
        }))
        .unwrap();

        assert_eq!(
            answer,
            Answer::Choice {
                name: Some("approved".to_string()),
                choice: ChoiceValue::Bool(true),
                confidence: 0.99,
                probabilities: vec![
                    ChoiceProbability {
                        value: ChoiceValue::Bool(true),
                        probability: 0.99
                    },
                    ChoiceProbability {
                        value: ChoiceValue::Bool(false),
                        probability: 0.01
                    }
                ]
            }
        );
    }

    #[test]
    fn defaults_missing_usage() {
        let decision: Decision = serde_json::from_value(json!({
            "model": "local-model",
            "answers": [{ "type": "predicate", "name": "ok", "probability": 1.0 }]
        }))
        .unwrap();

        assert_eq!(decision.usage, Usage::default());
    }

    #[tokio::test]
    async fn test_decision_request() {
        let mut server = mockito::Server::new_async().await;

        let response_body = r#"{
            "model": "gpt-6-luna",
            "answers": [
                {"type": "predicate", "name": "damaged", "probability": 0.95}
            ],
            "usage": {
                "input_tokens": 42,
                "input_tokens_details": {"cached_tokens": 0, "cache_write_tokens": 0},
                "output_tokens": 0,
                "output_tokens_details": {"reasoning_tokens": 0},
                "total_tokens": 42
            }
        }"#;

        let mock = server
            .mock("POST", "/v1/decisions")
            .match_header("authorization", "Bearer test-key")
            .match_body(mockito::Matcher::PartialJson(json!({
                "model": "gpt-6-luna",
                "input": "The package arrived with a broken screen.",
                "questions": [{
                    "type": "predicate",
                    "name": "damaged",
                    "instructions": "Does the customer report a damaged item?"
                }]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(response_body)
            .create();

        let input = DecisionInput::from("The package arrived with a broken screen.");
        let questions = vec![Question::Predicate {
            instructions: Instructions::from("Does the customer report a damaged item?"),
            name: Some("damaged".to_string()),
        }];

        let result = decision(
            server.url().as_str(),
            "test-key",
            "gpt-6-luna",
            &input,
            &questions,
        )
        .await;

        mock.assert();
        let decision = result.unwrap();
        assert_eq!(decision.model, "gpt-6-luna");
        assert_eq!(
            decision.answers[0],
            Answer::Predicate {
                name: Some("damaged".to_string()),
                probability: 0.95
            }
        );
    }

    #[tokio::test]
    async fn test_decision_with_options() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .match_body(mockito::Matcher::PartialJson(json!({
                "model": "local-model",
                "prompt_cache_key": "tenant-1",
                "prompt_cache_options": {"mode": "explicit", "ttl": "30m"},
                "safety_identifier": "user-1"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"local-model","answers":[]}"#)
            .create();

        let input = DecisionInput::from("evidence");
        let questions = vec![Question::Predicate {
            instructions: Instructions::from("Is it true?"),
            name: None,
        }];
        let options = DecisionOptions {
            prompt_cache_key: Some("tenant-1".to_string()),
            prompt_cache_options: Some(PromptCacheOptions {
                mode: Some(PromptCacheMode::Explicit),
                ttl: Some(PromptCacheTtl::ThirtyMinutes),
            }),
            safety_identifier: Some("user-1".to_string()),
        };

        let result = decision_with_options(
            server.url().as_str(),
            "test-key",
            "local-model",
            &options,
            &input,
            &questions,
        )
        .await;

        mock.assert();
        assert!(result.unwrap().answers.is_empty());
    }

    #[tokio::test]
    async fn test_decision_error_status() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(400)
            .with_body(r#"{"error":{"message":"bad request"}}"#)
            .create();

        let input = DecisionInput::from("evidence");
        let questions = vec![Question::Predicate {
            instructions: Instructions::from("Is it true?"),
            name: None,
        }];

        let result = decision(
            server.url().as_str(),
            "test-key",
            "gpt-6-luna",
            &input,
            &questions,
        )
        .await;

        mock.assert();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("400"));
        assert!(error.contains("bad request"));
    }

    #[tokio::test]
    async fn test_decision_trims_trailing_slash_from_host() {
        let mut server = mockito::Server::new_async().await;

        let mock = server
            .mock("POST", "/v1/decisions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"gpt-6-luna","answers":[]}"#)
            .create();

        let host = format!("{}/", server.url());
        let input = DecisionInput::from("evidence");
        let questions = vec![Question::Predicate {
            instructions: Instructions::from("Is it true?"),
            name: None,
        }];

        let result = decision(&host, "test-key", "gpt-6-luna", &input, &questions).await;

        mock.assert();
        assert!(result.is_ok());
    }
}
