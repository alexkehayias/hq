use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loop {
    pub id: String,
    /// Short, human-readable name used to identify the loop.
    pub title: String,
    /// Channels the loop subscribes to (its event sources).
    pub subscribe_channels: Vec<String>,
    /// Channels the loop is permitted to emit to, beyond any it created itself.
    pub emit_channels: Vec<String>,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Success,
    Error,
    Interrupted,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Success => "success",
            RunStatus::Error => "error",
            RunStatus::Interrupted => "interrupted",
        }
    }
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RunStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "running" => Ok(RunStatus::Running),
            "success" => Ok(RunStatus::Success),
            "error" => Ok(RunStatus::Error),
            "interrupted" => Ok(RunStatus::Interrupted),
            other => Err(format!("unknown run status: {other}")),
        }
    }
}

/// One event a loop processed, pointing at the background chat session that
/// holds its full transcript.
#[derive(Debug, Clone)]
pub struct LoopRun {
    pub id: String,
    pub loop_id: String,
    /// Title of the loop that produced this run, denormalized for display.
    pub loop_title: String,
    pub channel_id: String,
    pub event: String,
    pub status: RunStatus,
    pub session_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub response: Option<String>,
    pub error: Option<String>,
    /// Whether the transcript session still exists (it may have been pruned).
    pub has_transcript: bool,
}
