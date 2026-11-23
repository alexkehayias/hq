//! Tool wrapper around the OpenAI [Decisions API](crate::openai::decisions).
//!
//! The agent supplies the whole request — the shared `input`, one or more
//! `questions`, and optionally the `model` and prompt-caching/safety options.
//! The API hostname and key come from [`ToolConfig`] so the tool works against
//! both `api.openai.com` and a local OpenAI-compatible server.

use super::registry::{Tool, ToolConfig};
use crate::openai::decisions::{
    DEFAULT_MODEL, DecisionInput, DecisionOptions, PromptCacheOptions, Question,
    decision_with_options,
};
use crate::openai::{
    Function, Parameters, RecoverableToolError, ToolCall, ToolType, parse_tool_args,
};
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize)]
struct DecisionsArgs {
    #[serde(default)]
    model: Option<String>,
    input: DecisionInput,
    questions: Vec<Question>,
    #[serde(default)]
    prompt_cache_key: Option<String>,
    #[serde(default)]
    prompt_cache_options: Option<PromptCacheOptions>,
    #[serde(default)]
    safety_identifier: Option<String>,
}

#[derive(Serialize)]
pub struct DecisionTool {
    pub r#type: ToolType,
    pub function: Function<Value>,
    #[serde(skip)]
    api_hostname: String,
    #[serde(skip)]
    api_key: String,
}

#[async_trait]
impl ToolCall for DecisionTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: DecisionsArgs = parse_tool_args(args)?;

        if fn_args.questions.is_empty() {
            return Err(Error::from(RecoverableToolError::new(
                "At least one question is required.",
            )));
        }

        let options = DecisionOptions {
            prompt_cache_key: fn_args.prompt_cache_key,
            prompt_cache_options: fn_args.prompt_cache_options,
            safety_identifier: fn_args.safety_identifier,
        };
        let model = fn_args.model.as_deref().unwrap_or(DEFAULT_MODEL);

        let decision = decision_with_options(
            &self.api_hostname,
            &self.api_key,
            model,
            &options,
            &fn_args.input,
            &fn_args.questions,
        )
        .await?;

        Ok(serde_json::to_string(&decision)?)
    }

    fn function_name(&self) -> String {
        Self::NAME.to_string()
    }
}

impl Tool for DecisionTool {
    const NAME: &'static str = "decisions";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        Ok(Self::new(&conf.api_hostname, &conf.api_key))
    }
}

impl DecisionTool {
    pub fn new(api_hostname: &str, api_key: &str) -> Self {
        let function = Function {
            name: Self::NAME.to_string(),
            description: String::from(
                "Ask one or more typed questions about shared evidence using the Decisions API. \
                 Each predicate question returns a probability, each choice question returns \
                 the selected value, and each score question returns a score against ordered \
                 levels. Use it to classify, judge, or rate text or a conversation transcript. \
                 Answers come back in the same order as the questions, along with the model \
                 name and token usage.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: properties(),
                required: vec![String::from("input"), String::from("questions")],
                additional_properties: false,
            },
            strict: false,
        };
        Self {
            r#type: ToolType::Function,
            function,
            api_hostname: api_hostname.to_string(),
            api_key: api_key.to_string(),
        }
    }
}

impl Default for DecisionTool {
    fn default() -> Self {
        Self::new("https://api.openai.com", "")
    }
}

/// The JSON schema for the tool's arguments.
fn properties() -> Value {
    json!({
        "model": {
            "type": "string",
            "description": format!(
                "The Decisions model to use. Defaults to `{DEFAULT_MODEL}`."
            ),
        },
        "input": input_schema(),
        "questions": {
            "type": "array",
            "minItems": 1,
            "description": "Questions to ask about the input. Answers are returned in the same order.",
            "items": question_schema(),
        },
        "prompt_cache_key": {
            "type": "string",
            "description": "Optional key that groups related requests for prompt caching.",
        },
        "prompt_cache_options": {
            "type": "object",
            "description": "Optional prompt-caching settings.",
            "properties": {
                "mode": { "type": "string", "enum": ["implicit", "explicit"] },
                "ttl": { "type": "string", "enum": ["30m"] },
            },
            "additionalProperties": false,
        },
        "safety_identifier": {
            "type": "string",
            "description": "Optional stable identifier for the end user, used for safety monitoring.",
        },
    })
}

/// Shared evidence: plain text or an ordered conversation.
fn input_schema() -> Value {
    json!({
        "description": "Shared evidence every question is evaluated against: a plain text string, \
                        or an ordered array of conversation items.",
        "anyOf": [
            { "type": "string" },
            { "type": "array", "items": input_item_schema() },
        ],
    })
}

/// A conversation item in the `input` evidence array.
fn input_item_schema() -> Value {
    json!({
        "type": "object",
        "description": "A conversation item. `type` selects the shape: `message`, \
                        `function_call`, `function_call_output`, `custom_tool_call`, or \
                        `custom_tool_call_output`.",
        "properties": {
            "type": {
                "type": "string",
                "enum": [
                    "message",
                    "function_call",
                    "function_call_output",
                    "custom_tool_call",
                    "custom_tool_call_output"
                ],
            },
            "role": {
                "type": "string",
                "enum": ["system", "developer", "user", "assistant", "tool"],
                "description": "The message role (message items only).",
            },
            "content": message_content_schema(),
            "refusal": {
                "type": "boolean",
                "description": "Whether the message is a refusal (message items only).",
            },
            "tool_call_id": {
                "type": "string",
                "description": "The call id a tool-role message responds to.",
            },
            "tool_calls": {
                "type": "array",
                "description": "Function calls requested by an assistant message.",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "type": { "type": "string", "enum": ["function"] },
                        "function": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "arguments": { "type": "string" },
                            },
                            "required": ["name", "arguments"],
                            "additionalProperties": false,
                        },
                    },
                    "required": ["id", "type", "function"],
                    "additionalProperties": false,
                },
            },
            "phase": {
                "type": "string",
                "enum": ["commentary", "final_answer"],
                "description": "The phase of an assistant message.",
            },
            "call_id": { "type": "string" },
            "name": { "type": "string" },
            "arguments": { "type": "string" },
            "input": { "type": "string" },
            "output": {
                "description": "A tool output: a string or an array of content parts.",
                "anyOf": [
                    { "type": "string" },
                    { "type": "array", "items": content_part_schema() },
                ],
            },
            "id": { "type": "string" },
            "namespace": { "type": "string" },
            "status": {
                "type": "string",
                "enum": ["in_progress", "completed", "incomplete"],
            },
        },
        "required": ["type"],
        "additionalProperties": false,
    })
}

/// Message content: plain text or an array of content parts.
fn message_content_schema() -> Value {
    json!({
        "anyOf": [
            { "type": "string" },
            { "type": "array", "items": content_part_schema() },
        ],
    })
}

/// A content part used in Decisions evidence.
fn content_part_schema() -> Value {
    json!({
        "type": "object",
        "description": "A content part. The `type` field selects the shape.",
        "properties": {
            "type": {
                "type": "string",
                "enum": ["input_text", "input_image", "input_json", "output_text", "refusal"],
            },
            "text": {
                "type": "string",
                "description": "Text for input_text and output_text parts.",
            },
            "image_url": {
                "type": "string",
                "description": "A URL or data URL for an input_image part.",
            },
            "detail": {
                "type": "string",
                "enum": ["low", "high", "auto", "original"],
            },
            "data": {
                "description": "Arbitrary JSON for an input_json part.",
            },
            "refusal": {
                "type": "string",
                "description": "Refusal text for a refusal part.",
            },
            "prompt_cache_breakpoint": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["explicit"] },
                },
                "required": ["mode"],
                "additionalProperties": false,
            },
        },
        "required": ["type"],
        "additionalProperties": false,
    })
}

/// A question to ask about the input.
fn question_schema() -> Value {
    json!({
        "description": "A question about the input. Answers are returned in the same order.",
        "anyOf": [
            {
                "type": "object",
                "description": "Estimate the probability that a statement about the input is true.",
                "properties": {
                    "type": { "type": "string", "enum": ["predicate"] },
                    "instructions": instructions_schema(),
                    "name": {
                        "type": "string",
                        "description": "Optional name echoed back on the answer.",
                    },
                },
                "required": ["type", "instructions"],
                "additionalProperties": false,
            },
            {
                "type": "object",
                "description": "Choose one of the supplied options based on the input.",
                "properties": {
                    "type": { "type": "string", "enum": ["choice"] },
                    "instructions": instructions_schema(),
                    "choices": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "value": {
                                    "anyOf": [
                                        { "type": "string" },
                                        { "type": "boolean" },
                                    ],
                                },
                                "description": { "type": "string" },
                            },
                            "required": ["value"],
                            "additionalProperties": false,
                        },
                    },
                    "name": {
                        "type": "string",
                        "description": "Optional name echoed back on the answer.",
                    },
                },
                "required": ["type", "instructions", "choices"],
                "additionalProperties": false,
            },
            {
                "type": "object",
                "description": "Rate the input against the supplied ordered levels.",
                "properties": {
                    "type": { "type": "string", "enum": ["score"] },
                    "instructions": instructions_schema(),
                    "levels": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string" },
                                "description": { "type": "string" },
                            },
                            "required": ["label"],
                            "additionalProperties": false,
                        },
                    },
                    "name": {
                        "type": "string",
                        "description": "Optional name echoed back on the answer.",
                    },
                },
                "required": ["type", "instructions", "levels"],
                "additionalProperties": false,
            },
        ],
    })
}

/// A question's instructions: plain text or an array of content parts.
fn instructions_schema() -> Value {
    json!({
        "description": "The question's instructions, as plain text or an array of content parts.",
        "anyOf": [
            { "type": "string" },
            { "type": "array", "items": content_part_schema() },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn function_json_exposes_every_api_field() {
        let tool = DecisionTool::default();
        let value = serde_json::to_value(&tool.function).unwrap();

        assert_eq!(value["name"], "decisions");
        assert_eq!(
            value["parameters"]["required"],
            json!(["input", "questions"])
        );

        let properties = &value["parameters"]["properties"];
        for field in [
            "model",
            "input",
            "questions",
            "prompt_cache_key",
            "prompt_cache_options",
            "safety_identifier",
        ] {
            assert!(
                properties.get(field).is_some(),
                "missing property '{field}'"
            );
        }

        let question_types = properties["questions"]["items"]["anyOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|q| q["properties"]["type"]["enum"][0].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(question_types, vec!["predicate", "choice", "score"]);
    }

    #[tokio::test]
    async fn sends_the_request_and_returns_the_decision() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/decisions")
            .match_header("authorization", "Bearer test-key")
            .match_body(mockito::Matcher::PartialJson(json!({
                "model": "gpt-6-luna",
                "input": "The package arrived with a broken screen.",
                "questions": [{
                    "type": "predicate",
                    "instructions": "Does the customer report a damaged item?",
                    "name": "damaged"
                }],
                "prompt_cache_key": "tenant-1",
                "safety_identifier": "user-1"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "model": "gpt-6-luna",
                    "answers": [
                        {"type": "predicate", "name": "damaged", "probability": 0.95}
                    ]
                }"#,
            )
            .create();

        let tool = DecisionTool::new(server.url().as_str(), "test-key");
        let result = tool
            .call(
                r#"{
                    "input": "The package arrived with a broken screen.",
                    "questions": [{
                        "type": "predicate",
                        "instructions": "Does the customer report a damaged item?",
                        "name": "damaged"
                    }],
                    "prompt_cache_key": "tenant-1",
                    "safety_identifier": "user-1"
                }"#,
            )
            .await
            .unwrap();

        mock.assert();
        let decision: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(decision["answers"][0]["probability"], 0.95);
        assert_eq!(decision["model"], "gpt-6-luna");
    }

    #[tokio::test]
    async fn uses_the_supplied_model() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/decisions")
            .match_body(mockito::Matcher::PartialJson(json!({
                "model": "local-model"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"local-model","answers":[]}"#)
            .create();

        let tool = DecisionTool::new(server.url().as_str(), "test-key");
        tool.call(
            r#"{
                "model": "local-model",
                "input": "evidence",
                "questions": [{"type": "predicate", "instructions": "Is it true?"}]
            }"#,
        )
        .await
        .unwrap();

        mock.assert();
    }

    #[tokio::test]
    async fn rejects_empty_questions() {
        let tool = DecisionTool::default();
        let err = tool
            .call(r#"{"input": "evidence", "questions": []}"#)
            .await
            .unwrap_err();

        assert!(
            err.downcast_ref::<RecoverableToolError>()
                .is_some_and(|e| e.message.contains("At least one question"))
        );
    }

    #[tokio::test]
    async fn accepts_conversation_input_items() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/decisions")
            .match_body(mockito::Matcher::PartialJson(json!({
                "input": [
                    {"type": "message", "role": "user", "content": "What is the order status?"},
                    {
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "lookup_order",
                        "arguments": "{\"id\":42}",
                        "status": "completed"
                    }
                ]
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"gpt-6-luna","answers":[]}"#)
            .create();

        let tool = DecisionTool::new(server.url().as_str(), "test-key");
        tool.call(
            r#"{
                "input": [
                    {"type": "message", "role": "user", "content": "What is the order status?"},
                    {
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "lookup_order",
                        "arguments": "{\"id\":42}",
                        "status": "completed"
                    }
                ],
                "questions": [{"type": "predicate", "instructions": "Was the order looked up?"}]
            }"#,
        )
        .await
        .unwrap();

        mock.assert();
    }
}
