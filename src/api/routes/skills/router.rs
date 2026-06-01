//! Router for the skills API

use std::sync::{Arc, RwLock};

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use tokio::fs;
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
) -> Result<impl IntoResponse, ApiError> {
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
) -> Result<impl IntoResponse, ApiError> {
    let registry = state.read().expect("Unable to read shared state");

    let registry = match registry.skill_registry.as_ref() {
        Some(r) => r,
        None => return Ok((StatusCode::NOT_FOUND, Json(json!({"error": "Skills directory not configured"}))).into_response()),
    };

    let skill = match registry.load_skill(&name) {
        Ok(s) => s,
        Err(_) => return Ok((StatusCode::NOT_FOUND, Json(json!({"error": format!("Skill '{}' not found", name)}))).into_response()),
    };

    Ok(Json(SkillDetailResponse {
        name: skill.frontmatter.name,
        description: skill.frontmatter.description,
        license: skill.frontmatter.license,
        compatibility: skill.frontmatter.compatibility,
        metadata: skill.frontmatter.metadata,
        allowed_tools: skill.frontmatter.allowed_tools,
        body: skill.body,
    })
    .into_response())
}

/// List all files in a skill's directory (recursive).
async fn list_skill_files(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let skill_path = {
        let registry = state.read().expect("Unable to read shared state");

        let registry = match registry.skill_registry.as_ref() {
            Some(r) => r,
            None => {
                return Ok((StatusCode::NOT_FOUND, Json(json!({"error": "Skills directory not configured"}))).into_response())
            }
        };

        let skill = match registry.load_skill(&name) {
            Ok(s) => s,
            Err(_) => {
                return Ok((StatusCode::NOT_FOUND, Json(json!({"error": format!("Skill '{}' not found", name)}))).into_response())
            }
        };

        skill.path.clone()
    };

    let mut files = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        walk_directory(&skill_path, &skill_path, &mut files)?;
        Ok::<_, std::io::Error>(files)
    })
    .await
    .map_err(|e| ApiError::from(anyhow::anyhow!("Directory listing task failed: {}", e)))?
    .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to list files: {}", e)))?;

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

    Ok(Json(SkillFileListResponse { files }).into_response())
}

/// Read a file from a skill's directory.
async fn read_skill_file(
    State(state): State<SharedState>,
    Path((name, file_path)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let registry = state.read().expect("Unable to read shared state");

    let registry = match registry.skill_registry.as_ref() {
        Some(r) => r,
        None => return Ok((StatusCode::NOT_FOUND, Json(json!({"error": "Skills directory not configured"}))).into_response()),
    };

    let skill = match registry.load_skill(&name) {
        Ok(s) => s,
        Err(_) => return Ok((StatusCode::NOT_FOUND, Json(json!({"error": format!("Skill '{}' not found", name)}))).into_response()),
    };

    let content = match skill.read_file(&file_path) {
        Some(c) => c,
        None => {
            return Ok((StatusCode::NOT_FOUND, Json(json!({"error": format!("File '{}' not found in skill '{}'", file_path, name)}))).into_response())
        }
    };

    Ok(Json(SkillFileContentResponse {
        path: file_path,
        content,
    })
    .into_response())
}

/// Maximum file size for writes (1 MB).
const MAX_FILE_SIZE: usize = 1_048_576;

/// Write content to a file in a skill's directory.
async fn write_skill_file(
    State(state): State<SharedState>,
    Path((name, file_path)): Path<(String, String)>,
    Json(body): Json<SkillFileWriteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if body.content.len() > MAX_FILE_SIZE {
        return Ok((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error": format!("File too large. Maximum size is {} bytes", MAX_FILE_SIZE)})),
        )
            .into_response());
    }

    // Extract the skill's base path under the lock, then drop it before doing I/O
    let skill_path = {
        let state = state.read().expect("Unable to read shared state");

        let registry = match state.skill_registry.as_ref() {
            Some(r) => r,
            None => return Ok((StatusCode::NOT_FOUND, Json(json!({"error": "Skills directory not configured"}))).into_response()),
        };

        match registry.load_skill(&name) {
            Ok(s) => s.path,
            Err(_) => return Ok((StatusCode::NOT_FOUND, Json(json!({"error": format!("Skill '{}' not found", name)}))).into_response()),
        }
    };

    let full_path = skill_path.join(&file_path);

    // Security: prevent path traversal — ensure resolved path is within the skill directory
    let canonical_skill_dir = fs::canonicalize(&skill_path)
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to resolve skill path: {}", e)))?;

    // Create parent directories if they don't exist
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to create directories: {}", e)))?;
    }

    let canonical_file = match fs::canonicalize(&full_path).await {
        Ok(p) => p,
        Err(_) => {
            // File doesn't exist yet; canonicalize the parent and reconstruct
            let parent = full_path.parent().and_then(|p| {
                // Use std::fs::canonicalize in the sync fallback path
                p.canonicalize().ok()
            });
            match parent {
                Some(p) => p.join(full_path.file_name().unwrap()),
                None => {
                    return Err(ApiError::from(anyhow::anyhow!(
                        "Failed to resolve file path: invalid path"
                    )))
                }
            }
        }
    };

    if !canonical_file.starts_with(&canonical_skill_dir) {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Path traversal detected"})),
        )
            .into_response());
    }

    fs::write(&full_path, &body.content)
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("Failed to write file: {}", e)))?;

    Ok(Json(json!({"success": true})).into_response())
}

/// Recursively walk a directory and collect file entries.
fn walk_directory(
    base: &std::path::Path,
    dir: &std::path::Path,
    files: &mut Vec<SkillFileEntry>,
) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
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
