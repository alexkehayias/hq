//! Tool wrapper around the OpenAI [Decisions API](crate::openai::decisions).
//!
//! The agent supplies the request — the shared `input`, one or more
//! `questions`, and optionally the prompt-caching/safety options. The API
//! hostname, key, and model come from [`ToolConfig`] so the tool works against
//! both `api.openai.com` and a local OpenAI-compatible server.

use super::registry::{Tool, ToolConfig};
use crate::openai::decisions::{
    DEFAULT_MODEL, DecisionInput, DecisionOptions, PromptCacheOptions, Question,
    decision_with_options,
};
use crate::openai::{
    Function, Parameters, RecoverableToolError, Schema, ToolCall, ToolType, parse_tool_args,
};
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct DecisionsArgs {
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
pub struct DecisionProps {
    input: Schema,
    questions: Schema,
    prompt_cache_key: Schema,
    prompt_cache_options: Schema,
    safety_identifier: Schema,
}

#[derive(Serialize)]
pub struct DecisionTool {
    pub r#type: ToolType,
    pub function: Function<DecisionProps>,
    #[serde(skip)]
    api_hostname: String,
    #[serde(skip)]
    api_key: String,
    /// Model used for every request.
    #[serde(skip)]
    decision_model: String,
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
        let decision = decision_with_options(
            &self.api_hostname,
            &self.api_key,
            &self.decision_model,
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
    const NAME: &'static str = "decision";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        Ok(Self::new(
            &conf.api_hostname,
            &conf.api_key,
            &conf.decision_model,
        ))
    }
}

impl DecisionTool {
    pub fn new(api_hostname: &str, api_key: &str, decision_model: &str) -> Self {
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
            decision_model: decision_model.to_string(),
        }
    }
}

impl Default for DecisionTool {
    fn default() -> Self {
        Self::new("https://api.openai.com", "", DEFAULT_MODEL)
    }
}

/// The JSON schema for the tool's arguments.
fn properties() -> DecisionProps {
    DecisionProps {
        input: input_schema(),
        questions: Schema::array(question_schema())
            .min_items(1)
            .describe("Questions to ask about the input. Answers are returned in the same order."),
        prompt_cache_key: Schema::string()
            .describe("Optional key that groups related requests for prompt caching."),
        prompt_cache_options: Schema::object(vec![
            ("mode", Schema::enum_strings(&["implicit", "explicit"])),
            ("ttl", Schema::enum_strings(&["30m"])),
        ])
        .describe("Optional prompt-caching settings."),
        safety_identifier: Schema::string()
            .describe("Optional stable identifier for the end user, used for safety monitoring."),
    }
}

/// Shared evidence: plain text or an ordered conversation.
fn input_schema() -> Schema {
    Schema::any_of(vec![Schema::string(), Schema::array(input_item_schema())]).describe(
        "Shared evidence every question is evaluated against: a plain text string, or an \
         ordered array of conversation items.",
    )
}

/// A conversation item in the `input` evidence array.
fn input_item_schema() -> Schema {
    Schema::object(vec![
        (
            "type",
            Schema::enum_strings(&[
                "message",
                "function_call",
                "function_call_output",
                "custom_tool_call",
                "custom_tool_call_output",
            ]),
        ),
        (
            "role",
            Schema::enum_strings(&["system", "developer", "user", "assistant", "tool"])
                .describe("The message role (message items only)."),
        ),
        ("content", message_content_schema()),
        (
            "refusal",
            Schema::boolean().describe("Whether the message is a refusal (message items only)."),
        ),
        (
            "tool_call_id",
            Schema::string().describe("The call id a tool-role message responds to."),
        ),
        (
            "tool_calls",
            Schema::array(
                Schema::object(vec![
                    ("id", Schema::string()),
                    ("type", Schema::enum_strings(&["function"])),
                    (
                        "function",
                        Schema::object(vec![
                            ("name", Schema::string()),
                            ("arguments", Schema::string()),
                        ])
                        .required(&["name", "arguments"]),
                    ),
                ])
                .required(&["id", "type", "function"]),
            )
            .describe("Function calls requested by an assistant message."),
        ),
        (
            "phase",
            Schema::enum_strings(&["commentary", "final_answer"])
                .describe("The phase of an assistant message."),
        ),
        ("call_id", Schema::string()),
        ("name", Schema::string()),
        ("arguments", Schema::string()),
        ("input", Schema::string()),
        (
            "output",
            Schema::any_of(vec![Schema::string(), Schema::array(content_part_schema())])
                .describe("A tool output: a string or an array of content parts."),
        ),
        ("id", Schema::string()),
        ("namespace", Schema::string()),
        (
            "status",
            Schema::enum_strings(&["in_progress", "completed", "incomplete"]),
        ),
    ])
    .required(&["type"])
    .describe(
        "A conversation item. `type` selects the shape: `message`, `function_call`, \
         `function_call_output`, `custom_tool_call`, or `custom_tool_call_output`.",
    )
}

/// Message content: plain text or an array of content parts.
fn message_content_schema() -> Schema {
    Schema::any_of(vec![Schema::string(), Schema::array(content_part_schema())])
}

/// A content part used in Decisions evidence.
fn content_part_schema() -> Schema {
    Schema::object(vec![
        (
            "type",
            Schema::enum_strings(&[
                "input_text",
                "input_image",
                "input_json",
                "output_text",
                "refusal",
            ]),
        ),
        (
            "text",
            Schema::string().describe("Text for input_text and output_text parts."),
        ),
        (
            "image_url",
            Schema::string().describe("A URL or data URL for an input_image part."),
        ),
        (
            "detail",
            Schema::enum_strings(&["low", "high", "auto", "original"]),
        ),
        (
            "data",
            Schema::any().describe("Arbitrary JSON for an input_json part."),
        ),
        (
            "refusal",
            Schema::string().describe("Refusal text for a refusal part."),
        ),
        (
            "prompt_cache_breakpoint",
            Schema::object(vec![("mode", Schema::enum_strings(&["explicit"]))]).required(&["mode"]),
        ),
    ])
    .required(&["type"])
    .describe("A content part. The `type` field selects the shape.")
}

/// A question to ask about the input.
fn question_schema() -> Schema {
    Schema::any_of(vec![
        Schema::object(vec![
            ("type", Schema::enum_strings(&["predicate"])),
            ("instructions", instructions_schema()),
            (
                "name",
                Schema::string().describe("Optional name echoed back on the answer."),
            ),
        ])
        .required(&["type", "instructions"])
        .describe("Estimate the probability that a statement about the input is true."),
        Schema::object(vec![
            ("type", Schema::enum_strings(&["choice"])),
            ("instructions", instructions_schema()),
            (
                "choices",
                Schema::array(
                    Schema::object(vec![
                        (
                            "value",
                            Schema::any_of(vec![Schema::string(), Schema::boolean()]),
                        ),
                        ("description", Schema::string()),
                    ])
                    .required(&["value"]),
                )
                .min_items(1),
            ),
            (
                "name",
                Schema::string().describe("Optional name echoed back on the answer."),
            ),
        ])
        .required(&["type", "instructions", "choices"])
        .describe("Choose one of the supplied options based on the input."),
        Schema::object(vec![
            ("type", Schema::enum_strings(&["score"])),
            ("instructions", instructions_schema()),
            (
                "levels",
                Schema::array(
                    Schema::object(vec![
                        ("label", Schema::string()),
                        ("description", Schema::string()),
                    ])
                    .required(&["label"]),
                )
                .min_items(1),
            ),
            (
                "name",
                Schema::string().describe("Optional name echoed back on the answer."),
            ),
        ])
        .required(&["type", "instructions", "levels"])
        .describe("Rate the input against the supplied ordered levels."),
    ])
    .describe("A question about the input. Answers are returned in the same order.")
}

/// A question's instructions: plain text or an array of content parts.
fn instructions_schema() -> Schema {
    Schema::any_of(vec![Schema::string(), Schema::array(content_part_schema())])
        .describe("The question's instructions, as plain text or an array of content parts.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn function_json_exposes_agent_supplied_fields() {
        let tool = DecisionTool::default();
        let value = serde_json::to_value(&tool.function).unwrap();

        assert_eq!(value["name"], "decision");
        assert_eq!(
            value["parameters"]["required"],
            json!(["input", "questions"])
        );

        let properties = &value["parameters"]["properties"];
        for field in [
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
        assert!(
            properties.get("model").is_none(),
            "the model is configured, not agent-supplied"
        );

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

        let tool = DecisionTool::new(server.url().as_str(), "test-key", DEFAULT_MODEL);
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
    async fn uses_the_configured_model_by_default() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/decisions")
            .match_body(mockito::Matcher::PartialJson(json!({
                "model": "configured-model"
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"model":"configured-model","answers":[]}"#)
            .create();

        let tool = DecisionTool::new(server.url().as_str(), "test-key", "configured-model");
        tool.call(
            r#"{
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

        let tool = DecisionTool::new(server.url().as_str(), "test-key", DEFAULT_MODEL);
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
