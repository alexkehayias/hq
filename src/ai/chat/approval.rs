//! In-memory registry of pending tool-call approval requests.
//!
//! When the chat loop needs to confirm a sensitive tool call with the
//! user, it emits an approval-request chunk over the SSE stream and
//! registers a pending request here. The HTTP endpoint that receives
//! the user's decision resolves it through `ApprovalRegistry::resolve`,
//! which wakes up the chat task waiting in `ApprovalRegistry::request`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::{oneshot, watch};

/// The user's response to an approval request.
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    /// User approved the tool call. The chat loop will execute it.
    Approved,
    /// User denied the tool call, with an optional reason that is
    /// surfaced back to the model as the tool response.
    Denied(String),
}

/// Key for tracking a pending approval. A session may have multiple
/// outstanding approvals if the model batches several tool calls.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ApprovalKey {
    pub session_id: String,
    pub request_id: String,
}

/// A shared, in-process registry of pending tool-call approvals.
///
/// The chat task registers a request here and awaits the result; the
/// HTTP endpoint that receives the user's decision resolves it. Both
/// sides share a single `Arc<ApprovalRegistry>` so that the registry
/// outlives any individual chat turn (a server restart still loses
/// pending approvals, but a chat-task lifetime does not).
pub struct ApprovalRegistry {
    /// Pending approval requests awaiting a user response. Each entry
    /// holds a oneshot sender that `request` awaits.
    pending: Mutex<HashMap<ApprovalKey, oneshot::Sender<ApprovalDecision>>>,
    /// How long to wait for a user response before auto-denying.
    timeout: Duration,
}

impl ApprovalRegistry {
    /// Create a new registry with the given approval-response timeout.
    pub fn new(timeout: Duration) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            timeout,
        }
    }

    /// Register a pending approval and block until the user responds
    /// or the timeout elapses.
    ///
    /// If `request` is called twice with the same key, the second
    /// call's sender overwrites the first; the first receiver will
    /// observe a `Closed` error and return a denial. Callers should
    /// generate a fresh `request_id` per request to avoid this.
    pub async fn request(&self, session_id: &str, request_id: &str) -> ApprovalDecision {
        let key = ApprovalKey {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending map poisoned").insert(key.clone(), tx);

        let result = tokio::time::timeout(self.timeout, rx).await;

        // Always clean up our entry so stale senders don't accumulate.
        // If `resolve` raced us to it, this is a no-op; if we timed out,
        // this drops the sender so any later `resolve` finds nothing.
        self.pending
            .lock()
            .expect("pending map poisoned")
            .remove(&key);

        match result {
            Ok(Ok(decision)) => decision,
            Ok(Err(_)) => ApprovalDecision::Denied(
                "Approval request was cancelled before a response arrived".to_string(),
            ),
            Err(_) => ApprovalDecision::Denied(format!(
                "Approval timed out after {} seconds",
                self.timeout.as_secs()
            )),
        }
    }

    /// Resolve a pending approval request. Returns `true` if a
    /// waiting receiver was found and notified, `false` if no pending
    /// request exists for this key (already resolved, expired, or
    /// never created).
    pub fn resolve(
        &self,
        session_id: &str,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> bool {
        let key = ApprovalKey {
            session_id: session_id.to_string(),
            request_id: request_id.to_string(),
        };
        let removed = self
            .pending
            .lock()
            .expect("pending map poisoned")
            .remove(&key);
        match removed {
            Some(tx) => {
                // If send fails, the receiver was already dropped (e.g.,
                // the chat task errored out). Nothing more we can do.
                let _ = tx.send(decision);
                true
            }
            None => false,
        }
    }

    /// Number of currently pending approval requests. Useful for
    /// diagnostics and tests.
    pub fn pending_count(&self) -> usize {
        self.pending
            .lock()
            .expect("pending map poisoned")
            .len()
    }
}

impl Default for ApprovalRegistry {
    fn default() -> Self {
        // 5 minutes is long enough that a user stepping away to think
        // can still respond, short enough that a forgotten request
        // doesn't hold chat tasks forever.
        Self::new(Duration::from_secs(300))
    }
}

/// A watch channel used to signal when the chat task has finished
/// awaiting an approval, so that callers waiting on it (e.g., a test)
/// can synchronize. Not part of the public API; used by tests.
#[allow(dead_code)]
pub(crate) fn approval_signal() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_request_and_resolve_happy_path() {
        let registry = ApprovalRegistry::new(Duration::from_secs(1));
        let session_id = "session_1";
        let request_id = "req_1";

        // Spawn the request waiter
        let registry_clone = std::sync::Arc::new(registry);
        let registry_for_wait =
            std::sync::Arc::<ApprovalRegistry>::clone(&registry_clone);
        let wait_handle = tokio::spawn(async move {
            registry_for_wait.request(session_id, request_id).await
        });

        // Give the waiter a moment to register
        sleep(Duration::from_millis(10)).await;

        // Resolve with approval
        let resolved = registry_clone.resolve(session_id, request_id, ApprovalDecision::Approved);
        assert!(resolved, "resolve should find the pending entry");

        let decision = wait_handle.await.expect("wait task panicked");
        assert!(matches!(decision, ApprovalDecision::Approved));
    }

    #[tokio::test]
    async fn test_resolve_no_pending_returns_false() {
        let registry = ApprovalRegistry::new(Duration::from_secs(1));
        let result =
            registry.resolve("missing_session", "missing_req", ApprovalDecision::Approved);
        assert!(!result, "resolve should return false when no pending entry");
    }

    #[tokio::test]
    async fn test_timeout_denies_after_deadline() {
        let registry = ApprovalRegistry::new(Duration::from_millis(50));
        let decision = registry
            .request("session_timeout", "req_timeout")
            .await;
        match decision {
            ApprovalDecision::Denied(msg) => {
                assert!(
                    msg.contains("timed out"),
                    "expected timeout message, got: {msg}"
                );
            }
            other => panic!("expected Denied on timeout, got {other:?}"),
        }

        // Entry should be cleaned up after timeout
        assert_eq!(registry.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_resolve_after_timeout_is_noop() {
        let registry = ApprovalRegistry::new(Duration::from_millis(20));
        // Let request time out
        let _ = registry.request("s", "r").await;
        // Now resolve — should return false (entry already cleaned up)
        let result = registry.resolve("s", "r", ApprovalDecision::Approved);
        assert!(!result, "resolve after timeout should be a no-op");
    }

    #[tokio::test]
    async fn test_pending_count_tracks_outstanding() {
        let registry = std::sync::Arc::new(ApprovalRegistry::new(Duration::from_secs(1)));
        assert_eq!(registry.pending_count(), 0);

        // Spawn two pending requests
        let r1 = registry.clone();
        let r2 = registry.clone();
        let h1 = tokio::spawn(async move { r1.request("s1", "r1").await });
        let h2 = tokio::spawn(async move { r2.request("s1", "r2").await });

        sleep(Duration::from_millis(10)).await;
        assert_eq!(registry.pending_count(), 2);

        // Resolve both
        registry.resolve("s1", "r1", ApprovalDecision::Approved);
        registry.resolve("s1", "r2", ApprovalDecision::Denied("no".to_string()));

        let _ = h1.await;
        let _ = h2.await;

        // After all requests resolved, count back to 0
        assert_eq!(registry.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_denial_reason_is_propagated() {
        let registry = std::sync::Arc::new(ApprovalRegistry::new(Duration::from_secs(1)));
        let r2 = registry.clone();
        let h = tokio::spawn(async move { r2.request("s", "r").await });

        sleep(Duration::from_millis(10)).await;
        registry
            .clone()
            .resolve("s", "r", ApprovalDecision::Denied("not allowed".to_string()));

        let decision = h.await.expect("wait task panicked");
        match decision {
            ApprovalDecision::Denied(msg) => assert_eq!(msg, "not allowed"),
            other => panic!("expected Denied with reason, got {other:?}"),
        }
    }
}