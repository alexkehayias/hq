use std::sync::{Arc, RwLock};

use serde::Deserialize;
use tokio_rusqlite::Connection;

use crate::ai::chat::ApprovalRegistry;
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
    /// In-memory registry of pending tool-call approval requests.
    /// Shared between the chat task (which awaits a decision) and
    /// the approval API endpoint (which resolves it). Lives for the
    /// lifetime of `AppState`, not any individual chat turn.
    pub approval_registry: Arc<ApprovalRegistry>,
}

impl AppState {
    pub fn new(
        db: Connection,
        config: AppConfig,
        skill_registry: SkillRegistry,
    ) -> Self {
        Self {
            latest_selection: None,
            db,
            config,
            skill_registry: Arc::new(RwLock::new(skill_registry)),
            approval_registry: Arc::new(ApprovalRegistry::default()),
        }
    }
}
