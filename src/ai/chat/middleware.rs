use std::collections::HashSet;

use crate::openai::{Message, Role};
use anyhow::Error;
use async_trait::async_trait;

/// The action a middleware wants to take after inspecting the transcript.
#[derive(Debug)]
pub enum MiddlewareAction {
    /// Proceed with tool call execution.
    Continue,
    /// Stop the chat with an error.
    StopWithError(Error),
    /// Reject the pending tool calls by returning tool call response
    /// messages. The chat loop inserts these into the transcript and
    /// continues, allowing the model to recover with a normal response.
    Reject(Vec<Message>),
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
    ) -> MiddlewareAction;
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
    ) -> MiddlewareAction {
        // Extract all tool call (name, args) pairs from assistant messages
        let tool_calls: Vec<(String, String)> = transcript
            .iter()
            .filter(|m| *m.role() == Role::Assistant)
            .filter_map(|m| {
                m.tool_calls.as_ref().map(|calls| {
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
                let rejection_msg = format!(
                    "Tool call rejected due to infinite loop: tool '{name}' called \
                     with the same arguments {n} times in a row.",
                    name = first.0,
                    n = self.max_repeats,
                );

                // Build rejection responses for the pending tool calls
                let rejection_msgs: Vec<Message> = transcript
                    .iter()
                    .last()
                    .and_then(|m| m.tool_calls.as_ref())
                    .map(|calls| {
                        calls
                            .iter()
                            .map(|call| {
                                Message::new_tool_call_response(&rejection_msg, &call.id)
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                return MiddlewareAction::Reject(rejection_msgs);
            }
        }

        MiddlewareAction::Continue
    }
}

/// A single security rule: block `target` whenever any tool listed in
/// `blocked_after` has been called earlier in the same chat session.
///
/// For example, a rule with `target = "view_website"` and
/// `blocked_after = ["search_notes"]` prevents the `view_website`
/// tool from running after `search_notes` has been used.
#[derive(Clone)]
pub struct ToolCallRule {
    pub target: String,
    pub blocked_after: Vec<String>,
}

/// Middleware that applies security rules to pending tool calls based
/// on the chat transcript.
///
/// Each rule describes a tool that should be blocked if certain other
/// tools have already been called earlier in the session. When a rule
/// triggers, the pending tool call is rejected with a tool response
/// message explaining why it was blocked — the chat loop then
/// continues, giving the model a chance to adapt.
///
/// Only tool calls from prior assistant messages are considered when
/// looking up previously used tools, so a request that batches the
/// target and the blocking tool in the same message does not trip
/// itself.
pub struct ToolSecurityMiddleware {
    rules: Vec<ToolCallRule>,
}

impl Default for ToolSecurityMiddleware {
    /// Returns a middleware with the default rule set: reject
    /// `view_website` after `search_notes`.
    fn default() -> Self {
        Self::new(vec![ToolCallRule {
            target: "view_website".to_string(),
            blocked_after: vec!["search_notes".to_string()],
        }])
    }
}

impl ToolSecurityMiddleware {
    /// Create a middleware with an explicit set of rules.
    pub fn new(rules: Vec<ToolCallRule>) -> Self {
        Self { rules }
    }
}

#[async_trait]
impl ToolCallMiddleware for ToolSecurityMiddleware {
    async fn before_tool_calls(&self, transcript: &[Message]) -> MiddlewareAction {
        // The pending tool calls live in the last assistant message.
        let Some(pending_msg) = transcript.last() else {
            return MiddlewareAction::Continue;
        };
        if *pending_msg.role() != Role::Assistant {
            return MiddlewareAction::Continue;
        }
        let Some(pending_calls) = pending_msg.tool_calls.as_ref() else {
            return MiddlewareAction::Continue;
        };
        if pending_calls.is_empty() {
            return MiddlewareAction::Continue;
        }

        // Collect tool names from prior assistant messages only —
        // everything before the pending message. A tool requested in
        // the same batch as the target doesn't count as "previously
        // used" since it hasn't run yet.
        let mut prior_tool_names: HashSet<String> = HashSet::new();
        for m in transcript.iter().take(transcript.len().saturating_sub(1)) {
            if *m.role() != Role::Assistant {
                continue;
            }
            if let Some(calls) = m.tool_calls.as_ref() {
                for c in calls {
                    prior_tool_names.insert(c.function.name.clone());
                }
            }
        }

        for rule in &self.rules {
            // Does the pending batch include this rule's target tool?
            let blocked_pending: Vec<&crate::openai::FunctionCall> = pending_calls
                .iter()
                .filter(|c| c.function.name == rule.target)
                .collect();
            if blocked_pending.is_empty() {
                continue;
            }
            // Did any of the blocked-after tools appear previously?
            let triggered = rule
                .blocked_after
                .iter()
                .any(|name| prior_tool_names.contains(name));
            if !triggered {
                continue;
            }
            let msg = format!(
                "Tool call rejected: '{}' cannot be called after '{}'.",
                rule.target,
                rule.blocked_after.join(", "),
            );
            let rejections: Vec<Message> = blocked_pending
                .iter()
                .map(|c| Message::new_tool_call_response(&msg, &c.id))
                .collect();
            return MiddlewareAction::Reject(rejections);
        }

        MiddlewareAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::FunctionCall;

    #[tokio::test]
    async fn test_single_tool_call_not_a_loop() {
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
        let action = detector.before_tool_calls(&transcript).await;
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

        let action = detector.before_tool_calls(&transcript).await;
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

        let action = detector.before_tool_calls(&transcript).await;
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

        let action = detector.before_tool_calls(&transcript).await;
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

        let action = detector.before_tool_calls(&transcript).await;
        match action {
            MiddlewareAction::Reject(msgs) => {
                assert!(!msgs.is_empty());
                assert_eq!(*msgs[0].role(), Role::Tool);
                assert!(
                    msgs[0]
                        .content
                        .as_ref()
                        .unwrap()
                        .contains("Tool call rejected due to infinite loop"),
                    "Expected rejection message, got: {:?}",
                    msgs[0].content
                );
            }
            _ => panic!("Expected Reject"),
        }
    }

    // --- ToolSecurityMiddleware tests ---

    fn view_website_call(id: &str) -> FunctionCall {
        FunctionCall {
            function: crate::openai::FunctionCallFn {
                name: "view_website".into(),
                arguments: r#"{"url":"https://example.com"}"#.into(),
            },
            id: id.into(),
            r#type: "function".into(),
        }
    }

    fn search_notes_call(id: &str) -> FunctionCall {
        FunctionCall {
            function: crate::openai::FunctionCallFn {
                name: "search_notes".into(),
                arguments: r#"{"query":"test"}"#.into(),
            },
            id: id.into(),
            r#type: "function".into(),
        }
    }

    #[tokio::test]
    async fn test_security_allows_view_website_when_no_notes_used() {
        let mw = ToolSecurityMiddleware::default();
        let transcript = vec![
            Message::new(Role::User, "check this site"),
            Message::new_tool_call_request(vec![view_website_call("call_1")]),
        ];
        let action = mw.before_tool_calls(&transcript).await;
        assert!(
            matches!(action, MiddlewareAction::Continue),
            "Expected Continue, got {:?}",
            action
        );
    }

    #[tokio::test]
    async fn test_security_rejects_view_website_after_search_notes() {
        let mw = ToolSecurityMiddleware::default();
        // First batch: search_notes was called and responded to
        let transcript = vec![
            Message::new(Role::User, "find my notes about X"),
            Message::new_tool_call_request(vec![search_notes_call("call_1")]),
            Message::new_tool_call_response("notes result", "call_1"),
            // Second batch: now the model wants to view a website
            Message::new_tool_call_request(vec![view_website_call("call_2")]),
        ];
        let action = mw.before_tool_calls(&transcript).await;
        match action {
            MiddlewareAction::Reject(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert_eq!(*msgs[0].role(), Role::Tool);
                let content = msgs[0].content.as_ref().expect("missing content");
                assert!(
                    content.contains("view_website"),
                    "rejection should name the blocked tool, got: {content}",
                );
            }
            other => panic!("Expected Reject, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_security_allows_unrelated_tool_after_search_notes() {
        let mw = ToolSecurityMiddleware::default();
        // search_notes was used, but the pending call is a different
        // tool that isn't subject to any rule — should be allowed.
        let transcript = vec![
            Message::new(Role::User, "do stuff"),
            Message::new_tool_call_request(vec![search_notes_call("call_1")]),
            Message::new_tool_call_response("notes result", "call_1"),
            Message::new_tool_call_request(vec![FunctionCall {
                function: crate::openai::FunctionCallFn {
                    name: "web_search".into(),
                    arguments: r#"{"query":"rust"}"#.into(),
                },
                id: "call_2".into(),
                r#type: "function".into(),
            }]),
        ];
        let action = mw.before_tool_calls(&transcript).await;
        assert!(
            matches!(action, MiddlewareAction::Continue),
            "Expected Continue for unrelated tool, got {action:?}",
        );
    }

    #[tokio::test]
    async fn test_security_concurrent_batch_does_not_trigger() {
        let mw = ToolSecurityMiddleware::default();
        // search_notes and view_website in the same pending batch —
        // notes hasn't been "used" yet, so view_website should pass.
        let transcript = vec![
            Message::new(Role::User, "do both at once"),
            Message::new_tool_call_request(vec![
                search_notes_call("call_1"),
                view_website_call("call_2"),
            ]),
        ];
        let action = mw.before_tool_calls(&transcript).await;
        assert!(
            matches!(action, MiddlewareAction::Continue),
            "Concurrent batch should not trigger the rule, got {action:?}",
        );
    }

    #[tokio::test]
    async fn test_security_rejects_only_matching_pending_calls() {
        let mw = ToolSecurityMiddleware::default();
        // Pending batch has both view_website (blocked) and an
        // unrelated tool. Only the matching call should be rejected.
        let transcript = vec![
            Message::new(Role::User, "go"),
            Message::new_tool_call_request(vec![search_notes_call("call_1")]),
            Message::new_tool_call_response("notes result", "call_1"),
            Message::new_tool_call_request(vec![
                view_website_call("call_2"),
                FunctionCall {
                    function: crate::openai::FunctionCallFn {
                        name: "web_search".into(),
                        arguments: r#"{"query":"rust"}"#.into(),
                    },
                    id: "call_3".into(),
                    r#type: "function".into(),
                },
            ]),
        ];
        let action = mw.before_tool_calls(&transcript).await;
        match action {
            MiddlewareAction::Reject(msgs) => {
                // Only one rejection: the view_website call
                assert_eq!(msgs.len(), 1);
                assert_eq!(
                    msgs[0].tool_call_id(),
                    Some("call_2"),
                    "expected rejection for call_2, got {:?}",
                    msgs[0].tool_call_id(),
                );
            }
            other => panic!("Expected Reject, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_security_custom_rules() {
        // Custom rule: block "send_email" after "web_search"
        let mw = ToolSecurityMiddleware::new(vec![ToolCallRule {
            target: "send_email".to_string(),
            blocked_after: vec!["web_search".to_string()],
        }]);

        let transcript = vec![
            Message::new(Role::User, "go"),
            Message::new_tool_call_request(vec![FunctionCall {
                function: crate::openai::FunctionCallFn {
                    name: "web_search".into(),
                    arguments: r#"{}"#.into(),
                },
                id: "call_1".into(),
                r#type: "function".into(),
            }]),
            Message::new_tool_call_response("search result", "call_1"),
            Message::new_tool_call_request(vec![FunctionCall {
                function: crate::openai::FunctionCallFn {
                    name: "send_email".into(),
                    arguments: r#"{}"#.into(),
                },
                id: "call_2".into(),
                r#type: "function".into(),
            }]),
        ];
        let action = mw.before_tool_calls(&transcript).await;
        assert!(
            matches!(action, MiddlewareAction::Reject(_)),
            "Expected Reject for custom rule, got {action:?}",
        );
    }

    #[tokio::test]
    async fn test_security_empty_transcript_continues() {
        let mw = ToolSecurityMiddleware::default();
        let action = mw.before_tool_calls(&[]).await;
        assert!(matches!(action, MiddlewareAction::Continue));
    }

    #[tokio::test]
    async fn test_security_no_pending_tool_calls_continues() {
        let mw = ToolSecurityMiddleware::default();
        // Last message has no tool_calls — middleware should not panic.
        let transcript = vec![
            Message::new(Role::User, "hi"),
            Message::new(Role::Assistant, "hello"),
        ];
        let action = mw.before_tool_calls(&transcript).await;
        assert!(matches!(action, MiddlewareAction::Continue));
    }
}
