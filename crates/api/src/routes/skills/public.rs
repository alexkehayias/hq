//! Public types for the skills API

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct SkillListResponse {
    pub skills: Vec<SkillSummaryResponse>,
}

#[derive(Serialize)]
pub struct SkillSummaryResponse {
    pub name: String,
    pub description: String,
}

#[derive(Serialize)]
pub struct SkillDetailResponse {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: Option<std::collections::HashMap<String, String>>,
    pub allowed_tools: Option<String>,
    pub body: String,
}

#[derive(Serialize)]
pub struct SkillFileEntry {
    pub path: String,
    pub is_directory: bool,
}

#[derive(Serialize)]
pub struct SkillFileListResponse {
    pub files: Vec<SkillFileEntry>,
}

#[derive(Serialize)]
pub struct SkillFileContentResponse {
    pub path: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct SkillFileWriteRequest {
    pub content: String,
}
