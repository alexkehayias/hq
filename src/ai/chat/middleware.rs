use crate::openai::{Message, Role};
use anyhow::{Error, Result};
use async_trait::async_trait;

/// The action a middleware wants to take after inspecting the transcript.
pub enum MiddlewareAction {
    /// Proceed with tool call execution.
    Continue,
    /// Stop the chat with an error.
    StopWithError(Error),
    /// Inject messages into the transcript, then stop with an error.
    StopWithModifications(Vec<Message>, Error),
    /// Reject the pending tool calls by returning rejection responses.
    /// Each entry is `(tool_call_id, rejection_message)`. The chat loop
    /// sends these back as tool call responses, allowing the model to
    /// recover with a normal response.
    Reject(Vec<(String, String)>),
}

/// Middleware that runs before each batch of tool calls.
///
/// Implementations receive the full chat transcript and can decide
/// whether to allow the tool calls to proceed, stop with an error, or
/// inject messages into the transcript before stopping.
///
/// Each middleware owns its own private state — no shared state is
/// provided beyond the transcript.
#[async_trait]
pub trait ToolCallMiddleware: Send + Sync {
    /// Called before a batch of tool calls is executed.
    ///
    /// `transcript` contains the full chat history up to and including
    /// the latest assistant message with the pending tool call requests.
    async fn before_tool_calls(
        &self,
        transcript: &[Message],
    ) -> Result<MiddlewareAction>;
}

/// Detects when the LLM repeatedly calls the same tool with the same
/// arguments without any user message in between.
///
/// This is a common failure mode where the model gets stuck in a loop
/// — e.g., calling `note_search` with the same query, getting the same
/// result, and calling it again instead of using the result.
pub struct InfiniteLoopDetector {
    max_repeats: usize,
}

impl InfiniteLoopDetector {
    /// Create a new detector that triggers after `max_repeats`
    /// consecutive identical tool calls.
    pub fn new(max_repeats: usize) -> Self {
        Self { max_repeats }
    }
}

#[async_trait]
impl ToolCallMiddleware for InfiniteLoopDetector {
    async fn before_tool_calls(
        &self,
        transcript: &[Message],
    ) -> Result<MiddlewareAction> {
        // Extract all tool call (name, args) pairs from assistant messages
        let tool_calls: Vec<(String, String)> = transcript
            .iter()
            .filter(|m| *m.role() == Role::Assistant)
            .filter_map(|m| {
                m.tool_calls().map(|calls| {
                    calls
                        .iter()
                        .map(|call| {
                            (call.function.name.clone(), call.function.arguments.clone())
                        })
                        .collect::<Vec<_>>()
                })
            })
            .flatten()
            .collect();

        // Check for consecutive repeats at the end of the transcript
        if tool_calls.len() >= self.max_repeats {
            let last_n = &tool_calls[tool_calls.len() - self.max_repeats..];
            let first = &last_n[0];
            if last_n.iter().all(|(name, args)| name == &first.0 && args == &first.1) {
                // Extract tool call IDs from the latest assistant message
                // (the pending tool calls that triggered the rejection)
                let ids: Vec<String> = transcript
                    .iter()
                    .last()
                    .and_then(|m| m.tool_calls())
                    .map(|calls| calls.iter().map(|c| c.id.clone()).collect())
                    .unwrap_or_default();

                let rejection_msg = format!(
                    "Tool call rejected due to infinite loop: tool '{name}' called \
                     with the same arguments {n} times in a row.",
                    name = first.0,
                    n = self.max_repeats,
                );
                return Ok(MiddlewareAction::Reject(
                    ids.into_iter().map(|id| (id, rejection_msg.clone())).collect(),
                ));
            }
        }

        Ok(MiddlewareAction::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::FunctionCall;

    #[tokio::test]
    async fn test_no_loop_allowed() {
        let detector = InfiniteLoopDetector::new(3);
        let transcript = vec![
            Message::new(Role::User, "search for something"),
            Message::new_tool_call_request(vec![FunctionCall {
                function: crate::openai::FunctionCallFn {
                    name: "search".into(),
                    arguments: r#"{"query": "test"}"#.into(),
                },
                id: "call_1".into(),
                r#type: "function".into(),
            }]),
        ];
        let action = detector.before_tool_calls(&transcript).await.unwrap();
        assert!(matches!(action, MiddlewareAction::Continue));
    }

    #[tokio::test]
    async fn test_detects_loop() {
        let detector = InfiniteLoopDetector::new(3);

        // Simulate 3 consecutive identical tool calls
        let tool_call = || FunctionCall {
            function: crate::openai::FunctionCallFn {
                name: "search".into(),
                arguments: r#"{"query": "test"}"#.into(),
            },
            id: "call_1".into(),
            r#type: "function".into(),
        };

        let transcript = vec![
            Message::new(Role::User, "search for something"),
            // First tool call
            Message::new_tool_call_request(vec![tool_call()]),
            Message::new_tool_call_response("result 1", "call_1"),
            // Second tool call (same)
            Message::new_tool_call_request(vec![tool_call()]),
            Message::new_tool_call_response("result 2", "call_1"),
            // Third tool call (same — should trigger)
            Message::new_tool_call_request(vec![tool_call()]),
        ];

        let action = detector.before_tool_calls(&transcript).await.unwrap();
        assert!(matches!(action, MiddlewareAction::Reject(_)));
    }

    #[tokio::test]
    async fn test_different_args_not_loop() {
        let detector = InfiniteLoopDetector::new(3);

        let tool_call_1 = FunctionCall {
            function: crate::openai::FunctionCallFn {
                name: "search".into(),
                arguments: r#"{"query": "test"}"#.into(),
            },
            id: "call_1".into(),
            r#type: "function".into(),
        };
        let tool_call_2 = FunctionCall {
            function: crate::openai::FunctionCallFn {
                name: "search".into(),
                arguments: r#"{"query": "different"}"#.into(),
            },
            id: "call_2".into(),
            r#type: "function".into(),
        };

        let transcript = vec![
            Message::new(Role::User, "search"),
            Message::new_tool_call_request(vec![tool_call_1.clone()]),
            Message::new_tool_call_response("result 1", "call_1"),
            Message::new_tool_call_request(vec![tool_call_2]),
            Message::new_tool_call_response("result 2", "call_2"),
            Message::new_tool_call_request(vec![tool_call_1.clone()]),
        ];

        let action = detector.before_tool_calls(&transcript).await.unwrap();
        assert!(matches!(action, MiddlewareAction::Continue));
    }

    #[tokio::test]
    async fn test_below_threshold_not_loop() {
        let detector = InfiniteLoopDetector::new(3);

        let tool_call = || FunctionCall {
            function: crate::openai::FunctionCallFn {
                name: "search".into(),
                arguments: r#"{"query": "test"}"#.into(),
            },
            id: "call_1".into(),
            r#type: "function".into(),
        };

        let transcript = vec![
            Message::new(Role::User, "search"),
            Message::new_tool_call_request(vec![tool_call()]),
            Message::new_tool_call_response("result 1", "call_1"),
            Message::new_tool_call_request(vec![tool_call()]),
        ];

        let action = detector.before_tool_calls(&transcript).await.unwrap();
        // Only 2 repeats, threshold is 3 — should continue
        assert!(matches!(action, MiddlewareAction::Continue));
    }

    #[tokio::test]
    async fn test_loop_detected_has_rejection() {
        let detector = InfiniteLoopDetector::new(2);

        let tool_call = || FunctionCall {
            function: crate::openai::FunctionCallFn {
                name: "search".into(),
                arguments: r#"{"query": "test"}"#.into(),
            },
            id: "call_1".into(),
            r#type: "function".into(),
        };

        let transcript = vec![
            Message::new(Role::User, "search"),
            Message::new_tool_call_request(vec![tool_call()]),
            Message::new_tool_call_response("result 1", "call_1"),
            Message::new_tool_call_request(vec![tool_call()]),
        ];

        let action = detector.before_tool_calls(&transcript).await.unwrap();
        match action {
            MiddlewareAction::Reject(rejections) => {
                assert!(!rejections.is_empty());
                let (id, msg) = &rejections[0];
                assert_eq!(id, "call_1");
                assert!(msg.contains("Tool call rejected due to infinite loop"));
            }
            _ => panic!("Expected Reject"),
        }
    }
}
