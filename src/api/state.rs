use std::sync::{Arc, RwLock};

use serde::Deserialize;
use tokio_rusqlite::Connection;

use crate::ai::skills::SkillRegistry;
use crate::api::routes::chat::stream::{ChatStreamRegistry, new_registry};
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
    // In-flight (and recently finished) chat streams, keyed by session id, so
    // a reconnecting client can resume a response mid-generation.
    pub streams: ChatStreamRegistry,
}

impl AppState {
    pub fn new(db: Connection, config: AppConfig, skill_registry: SkillRegistry) -> Self {
        Self {
            latest_selection: None,
            db,
            config,
            skill_registry: Arc::new(RwLock::new(skill_registry)),
            streams: new_registry(),
        }
    }
}
