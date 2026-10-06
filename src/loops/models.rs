use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loop {
    pub id: String,
    pub channels: Vec<String>,
    pub system_prompt: Option<String>,
    pub tools: Vec<String>,
    pub debounce_ms: i64,
    pub created_at: String,
}
