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
