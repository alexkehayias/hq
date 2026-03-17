//! The core models for managing a stateful chat with an LLM.
use crate::openai::Message;
use anyhow::Error;
use serde::{Deserialize, Serialize};

// TODO: Should there be an app specific `Message` object instead of
// building around OpenAI?

#[derive(Default)]
pub struct Transcript(Vec<Message>);

impl Transcript {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn new_with_messages(messages: Vec<Message>) -> Self {
        Self(messages)
    }

    pub fn messages(&self) -> Vec<Message> {
        self.0.clone()
    }

    pub fn push(&mut self, msg: Message) {
        self.0.push(msg)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Message> {
        self.0.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Message> {
        self.0.iter_mut()
    }
}

/// Session mode determines how messages are processed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SessionMode {
    /// Regular chat mode using OpenAI API
    #[default]
    Chat,
    /// Code agent mode using Claude Code CLI
    Code,
}


impl std::fmt::Display for SessionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionMode::Chat => write!(f, "chat"),
            SessionMode::Code => write!(f, "code"),
        }
    }
}

impl std::str::FromStr for SessionMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(serde_json::from_str(&format!("\"{s}\""))?)
    }
}

pub struct Session {
    pub id: String,
    pub mode: SessionMode,
}
