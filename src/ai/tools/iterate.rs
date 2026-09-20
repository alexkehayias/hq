use std::time::Duration;

use super::{Tool, ToolConfig};
use crate::ai::chat::ChatBuilder;
use crate::openai::{
    Function, Message, Parameters, Property, Role, ToolCall, ToolType, parse_tool_args,
};
use anyhow::{Error, Result};
use async_trait::async_trait;
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

/// Cap on the combined output returned to the parent agent so the result
/// never blows up the parent's context window.
const MAX_OUTPUT_CHARS: usize = 20_000;

/// Maximum number of chunk subagents to run concurrently.
const MAX_CONCURRENCY: usize = 5;

const DEFAULT_CHUNK_SIZE: usize = 5;
const DEFAULT_TIMEOUT_SECS: u64 = 90;

#[derive(Serialize)]
pub struct IterateProps {
    pub items: Property,
    pub task: Property,
    pub chunk_size: Property,
    pub timeout_secs: Property,
}

#[derive(Deserialize)]
pub struct IterateArgs {
    pub items: Vec<String>,
    pub task: String,
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_chunk_size() -> usize {
    DEFAULT_CHUNK_SIZE
}

fn default_timeout_secs() -> u64 {
    DEFAULT_TIMEOUT_SECS
}

/// Outcome of one chunk's subagent run. Each chunk is isolated so a
/// failure or timeout doesn't abort the rest of the batch.
enum ChunkOutcome {
    Ok(usize, String),
    Errored(usize, String),
    TimedOut(usize),
}

#[derive(Serialize)]
pub struct IterateTool {
    pub r#type: ToolType,
    pub function: Function<IterateProps>,
    #[serde(skip_serializing)]
    api_hostname: String,
    #[serde(skip_serializing)]
    api_key: String,
    #[serde(skip_serializing)]
    model: String,
}

#[async_trait]
impl ToolCall for IterateTool {
    async fn call(&self, args: &str) -> Result<String, Error> {
        let mut fn_args: IterateArgs = parse_tool_args(args)?;

        let chunk_size = fn_args.chunk_size.max(1);
        let chunk_timeout = Duration::from_secs(fn_args.timeout_secs.max(1));
        // Move items into chunks without cloning; items is owned and unused after.
        let chunks: Vec<Vec<String>> = fn_args
            .items
            .chunks_mut(chunk_size)
            .map(|c| c.iter_mut().map(std::mem::take).collect())
            .collect();

        if chunks.is_empty() {
            return Ok("No items were provided to iterate over.".to_string());
        }

        let outcomes: Vec<ChunkOutcome> = stream::iter(chunks.into_iter().enumerate())
            .map(|(i, chunk)| self.run_chunk(i, chunk, &fn_args.task, chunk_timeout))
            .buffered(MAX_CONCURRENCY)
            .collect()
            .await;

        Ok(combine_outcomes(outcomes))
    }

    fn function_name(&self) -> String {
        Self::NAME.to_string()
    }
}

impl Tool for IterateTool {
    const NAME: &'static str = "iterate";

    fn from_config(conf: &ToolConfig) -> Result<Self> {
        Ok(Self::new(&conf.api_hostname, &conf.api_key, &conf.model))
    }
}

impl IterateTool {
    pub fn new(api_hostname: &str, api_key: &str, model: &str) -> Self {
        let function = Function {
            name: Self::NAME.to_string(),
            description: String::from(
                "Split a list of items into chunks and process each chunk with an \
                 independent subagent, then combine the results into a single report.",
            ),
            parameters: Parameters {
                r#type: String::from("object"),
                properties: IterateProps {
                    items: Property {
                        r#type: String::from("array"),
                        description: String::from("The list of items to iterate over and process."),
                        r#enum: None,
                    },
                    task: Property {
                        r#type: String::from("string"),
                        description: String::from(
                            "The instruction for how to process each chunk of items.",
                        ),
                        r#enum: None,
                    },
                    chunk_size: Property {
                        r#type: String::from("integer"),
                        description: String::from(
                            "How many items to include per chunk (default 5).",
                        ),
                        r#enum: None,
                    },
                    timeout_secs: Property {
                        r#type: String::from("integer"),
                        description: String::from(
                            "Maximum seconds to allow each chunk's subagent to run (default 90).",
                        ),
                        r#enum: None,
                    },
                },
                required: vec![
                    String::from("items"),
                    String::from("task"),
                    String::from("chunk_size"),
                    String::from("timeout_secs"),
                ],
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

    async fn run_chunk(
        &self,
        index: usize,
        chunk: Vec<String>,
        task: &str,
        timeout: Duration,
    ) -> ChunkOutcome {
        let items = chunk.join("\n");
        let system = Message::new(Role::System, task);
        let user = Message::new(
            Role::User,
            &format!("Process the following items:\n\n{}", items),
        );

        let mut chat = ChatBuilder::new(&self.api_hostname, &self.api_key, &self.model)
            .transcript(vec![system])
            .build();

        match tokio::time::timeout(timeout, chat.next_msg(user)).await {
            Ok(Ok(msgs)) => {
                let mut text = String::new();
                for m in msgs.iter().filter_map(|m| m.content.as_ref()) {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(m);
                }
                ChunkOutcome::Ok(index, text)
            }
            Ok(Err(e)) => ChunkOutcome::Errored(index, e.to_string()),
            Err(_) => ChunkOutcome::TimedOut(index),
        }
    }
}

fn combine_outcomes(outcomes: Vec<ChunkOutcome>) -> String {
    let mut out = String::new();
    for outcome in outcomes {
        let (index, status, text) = match outcome {
            ChunkOutcome::Ok(index, text) => (index, "ok", text),
            ChunkOutcome::Errored(index, e) => (index, "error", format!("Subagent errored: {}", e)),
            ChunkOutcome::TimedOut(index) => {
                (index, "timed out", "Subagent timed out.".to_string())
            }
        };
        let header = format!("\n\n=== Chunk {} ({}) ===\n", index, status);
        let remaining = MAX_OUTPUT_CHARS.saturating_sub(out.len() + header.len());
        if remaining == 0 {
            out.push_str("\n\n[Results truncated: exceeded output limit.]");
            break;
        }
        out.push_str(&header);
        // Keep a prefix of an oversized section so a single huge result
        // still contributes content instead of being dropped entirely.
        let body: String = text.chars().take(remaining).collect();
        out.push_str(&body);
        if body.len() < text.len() {
            out.push_str("\n\n[Results truncated: exceeded output limit.]");
            break;
        }
    }
    out
}

impl Default for IterateTool {
    fn default() -> Self {
        Self::new("https://api.openai.com", "test", "gpt-4")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_with_defaults() {
        let args = parse_tool_args::<IterateArgs>(
            r#"{"items": ["a", "b", "c"], "task": "summarize each"}"#,
        )
        .unwrap();
        assert_eq!(args.items, vec!["a", "b", "c"]);
        assert_eq!(args.task, "summarize each");
        assert_eq!(args.chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(args.timeout_secs, DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn parse_args_explicit() {
        let args = parse_tool_args::<IterateArgs>(
            r#"{"items": ["a"], "task": "t", "chunk_size": 2, "timeout_secs": 10}"#,
        )
        .unwrap();
        assert_eq!(args.chunk_size, 2);
        assert_eq!(args.timeout_secs, 10);
    }

    #[test]
    fn combine_marks_status() {
        let out = combine_outcomes(vec![
            ChunkOutcome::Ok(0, "done".to_string()),
            ChunkOutcome::Errored(1, "boom".to_string()),
            ChunkOutcome::TimedOut(2),
        ]);
        assert!(out.contains("Chunk 0 (ok)"));
        assert!(out.contains("done"));
        assert!(out.contains("Chunk 1 (error)"));
        assert!(out.contains("Subagent errored: boom"));
        assert!(out.contains("Chunk 2 (timed out)"));
        assert!(out.contains("Subagent timed out."));
    }

    #[test]
    fn combine_truncates() {
        let big = "x".repeat(MAX_OUTPUT_CHARS);
        let out = combine_outcomes(vec![ChunkOutcome::Ok(0, big)]);
        assert!(out.contains("Results truncated"));
        // The oversized section is kept as a prefix rather than dropped.
        assert!(out.starts_with("\n\n=== Chunk 0 (ok) ==="));
        assert!(out.contains("x".repeat(100).as_str()));
        // Hard cap: output stays within the limit plus the marker.
        assert!(out.len() <= MAX_OUTPUT_CHARS + 64);
    }
}
