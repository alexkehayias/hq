use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loop {
    pub id: String,
    /// Channels the loop subscribes to (its event sources).
    pub subscribe_channels: Vec<String>,
    /// Channels the loop is permitted to emit to, beyond any it created itself.
    pub emit_channels: Vec<String>,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub created_at: String,
}
