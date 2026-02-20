//! Claude Code CLI wrapper for non-interactive mode.
//!
//! This module provides a way to interact with Claude Code (via `ccr code`) in
//! non-interactive mode, streaming JSON events back to the caller.

use anyhow::{anyhow, Result};
use futures::stream::BoxStream;
use serde::Deserialize;
use tokio::process::Command;
use uuid::Uuid;

/// Default tools allowed for Claude Code sessions
const DEFAULT_TOOLS: &[&str] = &["Read", "Edit", "Bash"];

/// A session for interacting with Claude Code CLI
#[derive(Debug)]
pub struct ClaudeCodeSession {
    session_id: Uuid,
    allowed_tools: Vec<String>,
}

/// Streaming events from Claude Code
/// These are the inner events when type is "stream_event"
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum StreamEvent {
    /// Start of a new message
    #[serde(rename = "message_start")]
    MessageStart,

    /// Start of a content block (text or tool_use)
    #[serde(rename = "content_block_start")]
    ContentBlockStart {
        content_block: ContentBlock,
    },

    /// Incremental update to a content block
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta {
        delta: Delta,
    },

    /// End of a content block
    #[serde(rename = "content_block_stop")]
    ContentBlockStop,

    /// Message-level updates (stop reason, usage)
    #[serde(rename = "message_delta")]
    MessageDelta {
        usage: Option<Usage>,
        delta: Option<MessageDeltaFields>,
    },

    /// End of the message
    #[serde(rename = "message_stop")]
    MessageStop,
}

/// Outer wrapper for streaming events from Claude Code CLI
#[derive(Deserialize, Debug)]
pub struct StreamEventWrapper {
    /// The type of message (stream_event, system, assistant, result)
    #[serde(rename = "type")]
    pub message_type: String,

    /// The inner event (when type is "stream_event")
    #[serde(default)]
    pub event: Option<StreamEvent>,
}

/// Content block information
#[derive(Deserialize, Debug)]
pub struct ContentBlock {
    /// Type of content block (text or tool_use)
    #[serde(rename = "type")]
    pub block_type: String,

    /// ID of the content block (for tool_use)
    pub id: Option<String>,

    /// Name of the tool (for tool_use blocks)
    pub name: Option<String>,
}

/// Delta updates for content blocks
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum Delta {
    /// Text content delta
    #[serde(rename = "text_delta")]
    TextDelta { text: String },

    /// Tool input JSON delta (partial JSON that accumulates)
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

/// Token usage information
#[derive(Deserialize, Debug)]
pub struct Usage {
    pub input_tokens: u32,
    #[serde(rename = "output_tokens")]
    pub output_tokens: u32,
}

/// Message delta fields (stop reason, etc.)
#[derive(Deserialize, Debug)]
pub struct MessageDeltaFields {
    #[serde(rename = "stop_reason")]
    pub stop_reason: Option<String>,
}

/// Final result from Claude Code after message_stop
#[derive(Deserialize, Debug)]
pub struct ClaudeCodeResult {
    /// The final result text
    pub result: Option<String>,

    /// Session ID for continuing the conversation
    pub session_id: String,

    /// Type of result (should be "result")
    #[serde(rename = "type")]
    pub result_type: String,

    /// Subtype (e.g., "success", "error")
    pub subtype: Option<String>,

    /// Whether this is an error result
    #[serde(rename = "is_error")]
    pub is_error: bool,
}

impl ClaudeCodeSession {
    /// Create a new session with the given UUID and allowed tools
    pub fn new(session_id: Uuid, allowed_tools: Vec<String>) -> Self {
        Self {
            session_id,
            allowed_tools,
        }
    }

    /// Create a new session with default tools (Read, Edit, Bash)
    pub fn with_default_tools(session_id: Uuid) -> Self {
        Self {
            session_id,
            allowed_tools: DEFAULT_TOOLS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Get the session ID
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Get the allowed tools for this session
    pub fn allowed_tools(&self) -> &[String] {
        &self.allowed_tools
    }

    /// Start a new conversation with the given prompt
    ///
    /// Returns a stream of events that can be used to receive incremental updates
    pub fn start(&self, prompt: &str) -> BoxStream<'static, Result<StreamEvent>> {
        self.execute(prompt, false)
    }

    /// Continue an existing conversation with the given prompt
    ///
    /// Returns a stream of events that can be used to receive incremental updates
    pub fn resume(&self, prompt: &str) -> BoxStream<'static, Result<StreamEvent>> {
        self.execute(prompt, true)
    }

    /// Execute a prompt, optionally resuming an existing session
    fn execute(&self, prompt: &str, resume: bool) -> BoxStream<'static, Result<StreamEvent>> {
        let session_id = self.session_id;
        let tools = self.allowed_tools.clone();
        let prompt = prompt.to_string();

        Box::pin(async_stream::try_stream! {
            let mut cmd = Command::new("ccr");
            cmd.arg("code")
                .arg("--output-format")
                .arg("stream-json")
                .arg("--verbose")
                .arg("--include-partial-messages");

            if resume {
                cmd.arg("--resume").arg(session_id.to_string());
            } else {
                cmd.arg("--session-id").arg(session_id.to_string());
            }

            let tools_arg = tools.join(",");
            cmd.arg("--allowedTools").arg(&tools_arg)
                .arg("-p")
                .arg(prompt.as_str());

            // Capture stdout and stderr for debugging
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            tracing::debug!("Executing: ccr code with args: {:?}", cmd);

            let mut child = cmd.spawn()?;

            // Read stdout line by line
            use tokio::io::{AsyncBufReadExt, BufReader};
            let stdout = child.stdout.take().ok_or_else(|| {
                anyhow!("Failed to capture stdout from ccr process")
            })?;
            let mut lines = BufReader::new(stdout).lines();

            while let Some(line) = lines.next_line().await? {
                // Skip empty lines
                if line.trim().is_empty() {
                    continue;
                }

                // Parse as the wrapper type first to check if it's a stream_event
                match serde_json::from_str::<StreamEventWrapper>(&line) {
                    Ok(wrapper) => {
                        // If it's a stream_event with an inner event, yield that
                        if wrapper.message_type == "stream_event"
                            && let Some(event) = wrapper.event {
                                yield event;
                            }
                    }
                    Err(e) => {
                        // Try parsing as final result
                        match serde_json::from_str::<ClaudeCodeResult>(&line) {
                            Ok(result) => {
                                tracing::debug!("Received final result: is_error={}", result.is_error);
                            }
                            Err(_) => {
                                // Could be other output (like system init), log but don't fail
                                tracing::trace!("Failed to parse line as wrapper or result: {} - {}", e, line);
                            }
                        }
                    }
                }
            }

            // Wait for the process to complete
            let status = child.wait().await?;

            if !status.success() {
                tracing::warn!("ccr process exited with status: {}", status);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn test_default_tools() {
        let session = ClaudeCodeSession::with_default_tools(Uuid::new_v4());
        assert_eq!(session.allowed_tools(), vec!["Read", "Edit", "Bash"]);
    }

    #[test]
    fn test_custom_tools() {
        let session = ClaudeCodeSession::new(
            Uuid::new_v4(),
            vec!["Read".to_string(), "Bash".to_string()],
        );
        assert_eq!(session.allowed_tools(), vec!["Read", "Bash"]);
    }

    #[ignore]
    #[tokio::test]
    async fn test_claude_code_session() {
        let session = ClaudeCodeSession::with_default_tools(Uuid::new_v4());

        // Run a simple prompt
        let mut events = session.start("What programming language is this project written in?");

        let mut got_text = false;
        let mut text_content = String::new();
        let mut event_count = 0;

        while let Some(event_result) = events.next().await {
            let event = event_result.expect("Failed to get event");
            event_count += 1;

            match &event {
                StreamEvent::ContentBlockDelta { delta } => {
                    if let Delta::TextDelta { text } = delta {
                        got_text = true;
                        text_content.push_str(text);
                    }
                }
                StreamEvent::MessageStop => {
                    break;
                }
                _ => {
                    // Print first few events for debugging
                    if event_count <= 5 {
                        println!("Event {}: {:?}", event_count, event);
                    }
                }
            }
        }

        // Print what we got for debugging
        println!("Total events: {}", event_count);
        println!("Got text: {}", got_text);
        println!("Response: {}", text_content);

        // Verify we got some text response
        assert!(got_text, "Expected to receive text delta events");
        println!("Response: {}", text_content);

        // The response should mention Rust
        assert!(
            text_content.to_lowercase().contains("rust"),
            "Expected response to mention Rust, got: {}",
            text_content
        );
    }
}
