//! Public types for the webhook API
use serde::{Deserialize, Serialize};

/// Blurt notification from daemon
#[derive(Debug, Deserialize, Serialize)]
pub struct BlurtNotification {
    pub id: i64,
    pub title: String,
    pub subtitle: Option<String>,
    pub body: String,
    pub date: i64,
    pub bundle_id: Option<String>,
}

/// Minimal subset of GitHub's `push` webhook payload. Serde ignores the
/// remaining fields, so this stays stable across GitHub's payload changes.
#[derive(Debug, Deserialize)]
pub struct GithubPushEvent {
    /// Full git ref that was pushed, e.g. `refs/heads/main`.
    #[serde(rename = "ref")]
    pub git_ref: String,
}

/// Forwarded iOS notification
#[derive(Debug, Deserialize, Serialize)]
pub struct Notification {
    pub title: String,
    pub subtitle: Option<String>,
    pub message: String,
}

/// Token usage posted by lm-proxy after each proxied request.
///
/// Token buckets are normalized by the upstream provider so downstream
/// cost estimators can apply provider-specific rates (cache reads are
/// cheaper than fresh input on Anthropic; cache writes are billed at a
/// premium, etc.).
#[derive(Debug, Deserialize, Serialize)]
pub struct LmProxyMetric {
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u32>,
}
