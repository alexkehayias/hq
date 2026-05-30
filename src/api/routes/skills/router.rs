//! Router for the skills API

use std::fs;
use std::sync::{Arc, RwLock};

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde_json::json;

use super::public::{
    SkillDetailResponse, SkillFileContentResponse, SkillFileEntry, SkillFileListResponse,
    SkillFileWriteRequest, SkillListResponse, SkillSummaryResponse,
};
use crate::api::state::AppState;
use crate::api::public::ApiError;

type SharedState = Arc<RwLock<AppState>>;

/// List all available skills.
async fn list_skills(
    State(state): State<SharedState>,
) -> Result<Json<SkillListResponse>, ApiError> {
    let registry = state.read().expect("Unable to read shared state");

    let skills = match &registry.skill_registry {
        Some(registry) => registry
            .list_skills()
            .into_iter()
            .map(|s| SkillSummaryResponse {
                name: s.name,
                description: s.description,
            })
            .collect(),
        None => Vec::new(),
    };

    Ok(Json(SkillListResponse { skills }))
}

/// Get the full detail of a specific skill.
async fn get_skill_detail(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<SkillDetailResponse>, (StatusCode, Json<serde_json::Value>)> {
    let registry = state.read().expect("Unable to read shared state");

    let registry = registry
        .skill_registry
        .as_ref()
        .ok_or_else(|| not_found("Skills directory not configured"))?;

    let skill = registry
        .load_skill(&name)
        .map_err(|_| not_found(&format!("Skill '{}' not found", name)))?;

    Ok(Json(SkillDetailResponse {
        name: skill.frontmatter.name,
        description: skill.frontmatter.description,
        license: skill.frontmatter.license,
        compatibility: skill.frontmatter.compatibility,
        metadata: skill.frontmatter.metadata,
        allowed_tools: skill.frontmatter.allowed_tools,
        body: skill.body,
    }))
}

/// List all files in a skill's directory (recursive).
async fn list_skill_files(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<SkillFileListResponse>, (StatusCode, Json<serde_json::Value>)> {
    let registry = state.read().expect("Unable to read shared state");

    let registry = registry
        .skill_registry
        .as_ref()
        .ok_or_else(|| not_found("Skills directory not configured"))?;

    let skill = registry
        .load_skill(&name)
        .map_err(|_| not_found(&format!("Skill '{}' not found", name)))?;

    let mut files = Vec::new();

    // Walk the skill directory recursively
    walk_directory(&skill.path, &skill.path, &mut files)
        .map_err(|e| internal_error(&format!("Failed to list files: {}", e)))?;

    // Sort: SKILL.md first, then alphabetically
    files.sort_by(|a, b| {
        if a.path == "SKILL.md" {
            return std::cmp::Ordering::Less;
        }
        if b.path == "SKILL.md" {
            return std::cmp::Ordering::Greater;
        }
        a.path.cmp(&b.path)
    });

    Ok(Json(SkillFileListResponse { files }))
}

/// Read a file from a skill's directory.
async fn read_skill_file(
    State(state): State<SharedState>,
    Path((name, file_path)): Path<(String, String)>,
) -> Result<Json<SkillFileContentResponse>, (StatusCode, Json<serde_json::Value>)> {
    let registry = state.read().expect("Unable to read shared state");

    let registry = registry
        .skill_registry
        .as_ref()
        .ok_or_else(|| not_found("Skills directory not configured"))?;

    let skill = registry
        .load_skill(&name)
        .map_err(|_| not_found(&format!("Skill '{}' not found", name)))?;

    let content = skill
        .read_file(&file_path)
        .ok_or_else(|| not_found(&format!("File '{}' not found in skill '{}'", file_path, name)))?;

    Ok(Json(SkillFileContentResponse {
        path: file_path,
        content,
    }))
}

/// Write content to a file in a skill's directory.
async fn write_skill_file(
    State(state): State<SharedState>,
    Path((name, file_path)): Path<(String, String)>,
    Json(body): Json<SkillFileWriteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let registry = state.read().expect("Unable to read shared state");

    let registry = registry
        .skill_registry
        .as_ref()
        .ok_or_else(|| not_found("Skills directory not configured"))?;

    let skill = registry
        .load_skill(&name)
        .map_err(|_| not_found(&format!("Skill '{}' not found", name)))?;

    let full_path = skill.path.join(&file_path);

    // Security: prevent path traversal — ensure resolved path is within the skill directory
    let canonical_skill_dir = skill
        .path
        .canonicalize()
        .map_err(|e| internal_error(&format!("Failed to resolve skill path: {}", e)))?;

    // Create parent directories if they don't exist
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| internal_error(&format!("Failed to create directories: {}", e)))?;
    }

    let canonical_file = full_path
        .canonicalize()
        .or_else(|_| {
            // File doesn't exist yet; canonicalize the parent and reconstruct
            full_path
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .map(|p| p.join(full_path.file_name().unwrap()))
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "invalid path"))
        })
        .map_err(|e| internal_error(&format!("Failed to resolve file path: {}", e)))?;

    if !canonical_file.starts_with(&canonical_skill_dir) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Path traversal detected"})),
        ));
    }

    fs::write(&full_path, &body.content)
        .map_err(|e| internal_error(&format!("Failed to write file: {}", e)))?;

    Ok(Json(json!({"success": true})))
}

/// Recursively walk a directory and collect file entries.
fn walk_directory(
    base: &std::path::Path,
    dir: &std::path::Path,
    files: &mut Vec<SkillFileEntry>,
) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip hidden files/directories (starting with '.')
        if let Some(name) = path.file_name() {
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
        }

        let relative = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        if path.is_dir() {
            files.push(SkillFileEntry {
                path: relative,
                is_directory: true,
            });
            walk_directory(base, &path, files)?;
        } else if path.is_file() {
            files.push(SkillFileEntry {
                path: relative,
                is_directory: false,
            });
        }
    }
    Ok(())
}

fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": msg})),
    )
}

fn internal_error(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": msg})),
    )
}

/// Create the skills router.
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/", axum::routing::get(list_skills))
        .route("/{name}", axum::routing::get(get_skill_detail))
        .route("/{name}/files", axum::routing::get(list_skill_files))
        .route(
            "/{name}/files/{*path}",
            axum::routing::get(read_skill_file).put(write_skill_file),
        )
}
