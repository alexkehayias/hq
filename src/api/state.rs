use serde::Deserialize;
use tokio_rusqlite::Connection;

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
    pub skill_registry: Option<SkillRegistry>,
}

impl AppState {
    pub fn new(
        db: Connection,
        config: AppConfig,
        skill_registry: Option<SkillRegistry>,
    ) -> Self {
        Self {
            latest_selection: None,
            db,
            config,
            skill_registry,
        }
    }
}
