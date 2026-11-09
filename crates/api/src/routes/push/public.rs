//! Public types for the push API
use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct PushSubscriptionRequest {
    pub endpoint: String,
    pub keys: HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct NotificationRequest {
    pub message: String,
}
