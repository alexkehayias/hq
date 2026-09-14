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
#[derive(Debug, Deserialize, Serialize)]
pub struct GithubPushEvent {
    /// Full git ref that was pushed, e.g. `refs/heads/main`.
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub repository: Option<GithubRepository>,
}

/// Repository object from a GitHub webhook payload (only the fields we log).
#[derive(Debug, Deserialize, Serialize)]
pub struct GithubRepository {
    pub full_name: String,
}
