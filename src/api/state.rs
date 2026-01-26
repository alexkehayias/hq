use serde::Deserialize;
use tokio_rusqlite::Connection;

use crate::core::AppConfig;

#[derive(Debug, Deserialize)]
pub struct LastSelection {
    pub id: String,
    pub title: String,
    pub file_name: String,
}

pub struct AppState {
    // Stores the latest search hit selected by the user
    pub latest_selection: Option<LastSelection>,
    pub db: Connection,
    pub config: AppConfig,
}

impl AppState {
    pub fn new(db: Connection, config: AppConfig) -> Self {
        Self {
            latest_selection: None,
            db,
            config,
        }
    }
}
