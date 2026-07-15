use std::sync::{Arc, RwLock};

use serde::Deserialize;
use tokio_rusqlite::Connection;

use crate::ai::chat::ChatSessionManager;
use crate::ai::pubsub::PubSubBroker;
use crate::ai::skills::SkillRegistry;
use crate::core::AppConfig;

#[derive(Debug, Deserialize, Clone)]
pub struct LastSelection {
    pub id: String,
    pub title: String,
    pub file_name: String,
}

#[derive(Clone)]
pub struct AppState {
    // Stores the latest search hit selected by the user
    pub latest_selection: Option<LastSelection>,
    pub db: Connection,
    pub config: AppConfig,
    pub skill_registry: Arc<RwLock<SkillRegistry>>,
    /// In-memory pub/sub broker. Shared across the process so any
    /// handler, job, or tool can publish a [`crate::openai::Message`]
    /// to a named channel; chat sessions subscribe via
    /// [`ChatSessionManager`] and process received messages through
    /// their normal `next_msg` loop.
    pub pubsub: Arc<PubSubBroker>,
    /// Manages long-lived `ChatTask`s — one per chat session. Owns the
    /// in-memory transcript so HTTP requests and pub/sub messages share
    /// one conversation state. Spawns tasks lazily on first activity.
    pub chat_sessions: Arc<ChatSessionManager>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Connection,
        config: AppConfig,
        skill_registry: Arc<RwLock<SkillRegistry>>,
        pubsub: Arc<PubSubBroker>,
        chat_sessions: Arc<ChatSessionManager>,
    ) -> Self {
        Self {
            latest_selection: None,
            db,
            config,
            skill_registry,
            pubsub,
            chat_sessions,
        }
    }
}