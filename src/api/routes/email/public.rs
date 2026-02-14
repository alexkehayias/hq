//! Public types for the email API
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct EmailUnreadQuery {
    pub email: String,
    pub limit: Option<i64>,
}

#[derive(Clone, Serialize)]
pub struct EmailMessage {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub to: String,
    pub received: String,
    pub subject: String,
    pub body: String,
}

#[derive(Clone, Serialize)]
pub struct EmailThread {
    pub id: String,
    pub received: String,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub messages: Vec<EmailMessage>,
}
