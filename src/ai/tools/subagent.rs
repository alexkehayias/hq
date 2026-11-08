use std::time::Duration;

use super::{Tool, ToolConfig};
use crate::ai::chat::ChatBuilder;
use crate::openai::{
    Function, Message, Parameters, Property, Role, ToolCall, ToolType, parse_tool_args,
};
use anyhow::{Error, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Cap on the output returned to the parent agent so the result never blows up
/// the parent's context window.
const MAX_OUTPUT_CHARS: usize = 20_000;

const DEFAULT_TIMEOUT_SECS: u64 = 90;

/// The subagent gets a fresh transcript, so it needs to be told that it is
/// working from a delegated task rather than the parent's conversation.
const SYSTEM_PROMPT: &str = "You are a subagent. Another agent delegated a self-contained \
task to you because it is working on a larger goal. You do not see that agent's conversation \
or its tools, so work only from the task you are given. Complete the task and reply with your \
final result; the parent agent will use your response directly.";

#[derive(Serialize)]
pub struct SubagentProps {
    pub task: Property,
    pub timeout_secs: Property,
}

#[derive(Deserialize)]
pub struct SubagentArgs {
    pub task: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

#[derive(Serialize)]
pub struct SubagentTool {
    pub r#type: ToolType,
    pub function: Function<SubagentProps>,
    #[serde(skip_serializing)]
    api_hostname: String,
    #[serde(skip_serializing)]
    api_key: String,
    #[serde(skip_serializing)]
    model: String,
}

#[async_trait]
impl ToolCall for SubagentTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let fn_args: SubagentArgs = parse_tool_args(args)?;
        let timeout = Duration::from_secs(fn_args.timeout_secs.max(1));
        Ok(self.run_subagent(&fn_args.task, timeout).await)
    }

    fn function_name(&self) -> String {
        Self::NAME.to_string()
    }
}

impl Tool for SubagentTool {
    const NAME: &'static str = "subagent";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        Ok(Self::new(&conf.api_hostname, &conf.api_key, &conf.model))
    }
}

impl SubagentTool {
    pub fn new(api_hostname: &str, api_key: &str, model: &str) -> Self {
        let function = Function {
            name: Self::NAME.to_string(),
            description: String::from(
                "Delegate a self-contained task to an independent subagent and get its result \
                 back. The subagent has a fresh context and does not see this conversation, so \
                 the task must include everything it needs: background, the work to do, and the \
                 expected output. The subagent cannot call tools. Use it to parallelize \
                 independent work by issuing several subagent calls in the same turn, or to \
                 offload a focused subtask whose details you don't want to carry in this \
                 conversation.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: SubagentProps {
                    task: Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "A complete, self-contained task for the subagent. Include all \
                             context and inputs it needs, plus the expected output format.",
                        ),
                        r#enum: None,
                    },
                    timeout_secs: Property {
                        r#type: String::from("integer"),
                        description: String::from(
                            "Maximum seconds to allow the subagent to run (default 90).",
                        ),
                        r#enum: None,
                    },
                },
                required: vec![String::from("task"), String::from("timeout_secs")],
                additional_properties: false,
            },
            strict: true,
        };
        Self {
            r#type: ToolType::Function,
            function,
            api_hostname: api_hostname.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    /// Run one delegated task in an isolated chat. Failures and timeouts are
    /// returned as text rather than errors so the parent can react to them
    /// without aborting its turn.
    async fn run_subagent(&self, task: &str, timeout: Duration) -> String {
        let system = Message::new(Role::System, SYSTEM_PROMPT);
        let user = Message::new(Role::User, task);

        let mut chat = ChatBuilder::new(&self.api_hostname, &self.api_key, &self.model)
            .transcript(vec![system])
            .build();

        match tokio::time::timeout(timeout, chat.next_msg(user)).await {
            Ok(Ok(msgs)) => {
                let mut text = String::new();
                for m in msgs.iter().filter_map(|m| m.text()) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(m);
                }
                truncate_output(text)
            }
            Ok(Err(e)) => format!("Subagent errored: {}", e),
            Err(_) => String::from("Subagent timed out."),
        }
    }
}

fn truncate_output(text: String) -> String {
    if text.chars().count() <= MAX_OUTPUT_CHARS {
        return text;
    }
    let mut out: String = text.chars().take(MAX_OUTPUT_CHARS).collect();
    out.push_str("\n\n[Result truncated: exceeded output limit.]");
    out
}

impl Default for SubagentTool {
    fn default() -> Self {
        Self::new("https://api.openai.com", "test", "gpt-4")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_with_defaults() {
        let args = parse_tool_args::<SubagentArgs>(r#"{"task": "summarize this"}"#).unwrap();
        assert_eq!(args.task, "summarize this");
        assert_eq!(args.timeout_secs, DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn parse_args_explicit() {
        let args = parse_tool_args::<SubagentArgs>(r#"{"task": "t", "timeout_secs": 10}"#).unwrap();
        assert_eq!(args.timeout_secs, 10);
    }

    #[test]
    fn truncates_oversized_output() {
        let big = "x".repeat(MAX_OUTPUT_CHARS + 100);
        let out = truncate_output(big);
        assert!(out.contains("Result truncated"));
        assert!(out.starts_with("xxx"));
        assert!(out.chars().count() <= MAX_OUTPUT_CHARS + 64);
    }

    #[test]
    fn leaves_small_output_untouched() {
        assert_eq!(truncate_output("done".to_string()), "done");
    }
}
