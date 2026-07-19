//! Public types for the approval API

use serde::{Deserialize, Serialize};

/// Request body for `POST /api/approval/{session_id}`.
///
/// `request_id` must match a pending approval request that
/// `ApprovalMiddleware` registered with the shared registry. The
/// chat task waiting on it is woken up and either continues (if
/// approved) or rejects the tool call with a user-facing message.
#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub request_id: String,
    /// True to approve; false (or omitted) to deny. When `denied`,
    /// `message` is shown to the model as the tool response.
    pub approved: bool,
    /// Optional denial reason. Ignored when `approved` is true.
    #[serde(default)]
    pub message: Option<String>,
}

/// Response body for the approval endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApprovalResponse {
    /// True if a pending request was found and resolved.
    pub resolved: bool,
}

/// Error response shape (returned as JSON with a non-2xx status).
#[derive(Debug, Serialize)]
pub struct ApprovalError {
    pub error: String,
}