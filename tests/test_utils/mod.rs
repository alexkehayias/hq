//! Test utilities for integration tests
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::{Router, body::Body};

use hq::ai::chat::db::{get_or_create_session, insert_chat_message};
use hq::ai::skills::SkillRegistry;
use hq::api::AppState;
use hq::api::app;
use hq::core::AppConfig;
use hq::core::db::async_db;
use hq::core::db::initialize_db;
use hq::openai::{Message, Role};
use hq::search::{index_all, index_all_chat_sessions};

/// Converts a response body to a string
#[allow(dead_code)] // Otherwise test crates give dead code warning
pub async fn body_to_string(body: Body) -> String {
    let bytes = axum::body::to_bytes(body, 16384usize).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Creates a test application router with temporary directories.
///
/// Anything that uses this fixture can not be run in parallel due
/// to a lock held by `tantivy` during index writing so add a
/// `#[serial]` to the test function or run `cargo test --
/// --test-threads=1`.
#[allow(dead_code)] // Otherwise test crates give dead code warning
pub async fn test_app() -> Router {
    // Create a unique directory for the test using a UUID to avoid
    // collisions between tests running in the same second
    let temp_dir = env::temp_dir();
    let dir = temp_dir.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("Failed to create base directory");

    // Create the directory from each path
    let notes_path = dir.join("notes");
    let index_path = dir.join("index");
    let vec_db_path = dir.join("db");
    fs::create_dir_all(&notes_path).expect("Failed to create notes directory");
    fs::create_dir_all(&index_path).expect("Failed to create index directory");
    fs::create_dir_all(&vec_db_path).expect("Failed to create db directory");

    let db_path_str = dir.join(&vec_db_path);
    let db_path_str = db_path_str.to_str().unwrap();

    let db = async_db(db_path_str)
        .await
        .expect("Failed to connect to async db");
    db.call(|conn| {
        initialize_db(conn).expect("Failed to migrate db");
        Ok(())
    })
    .await
    .unwrap();

    index_dummy_notes_async(&db, dir.clone()).await;

    let app_config = AppConfig {
        notes_path: notes_path.display().to_string(),
        index_path: index_path.display().to_string(),
        vec_db_path: vec_db_path.to_str().unwrap().to_string(),
        storage_path: dir.display().to_string(),
        skills_path: String::from(""),
        deploy_key_path: String::from("test_deploy_key_path"),
        vapid_key_path: String::from("test_vapid_key_path"),
        note_search_api_url: String::from("http://localhost:2222"),
        gmail_api_client_id: String::from("test_client_id"),
        gmail_api_client_secret: String::from("test_client_secret"),
        google_search_api_key: String::from("test_google_search_key"),
        google_search_cx_id: String::from("test_cx_id"),
        openai_model: String::from("gpt-4o"),
        openai_api_hostname: String::from("https://api.openai.com"),
        openai_api_key: String::from("test-api-key"),
        system_message: String::from("You are a helpful assistant."),
    };
    let app_state = AppState::new(db, app_config, None);
    app(Arc::new(RwLock::new(app_state)))
}

async fn index_dummy_notes_async(db: &tokio_rusqlite::Connection, temp_dir: PathBuf) {
    let index_dir = temp_dir.join("index");
    let index_dir_path = index_dir.to_str().unwrap();
    fs::create_dir_all(index_dir_path).expect("Failed to create directory");

    let notes_dir = temp_dir.join("notes");
    let notes_dir_path = notes_dir.to_str().unwrap();
    fs::create_dir_all(notes_dir_path).expect("Failed to create directory");

    let test_note_path = notes_dir.join("test.org");
    let paths = vec![test_note_path.clone()];

    fs::write(
        test_note_path,
        r#":PROPERTIES:
:ID:       6A503659-15E4-4427-835F-7873F8FF8ECF
:END:
#+TITLE: this is a test
#+DATE: 2025-01-28
"#,
    )
    .unwrap();

    index_all(db, index_dir_path, notes_dir_path, true, true, Some(paths))
        .await
        .unwrap();
}

/// Creates a test application router and returns the state for direct DB access.
/// This is primarily used for tests that need to insert data directly.
#[allow(dead_code)]
pub async fn test_app_with_state() -> (Router, AppState) {
    // Create a unique directory for the test using a UUID to avoid
    // collisions between tests running in the same second
    let temp_dir = env::temp_dir();
    let dir = temp_dir.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("Failed to create base directory");

    // Create the directory from each path
    let notes_path = dir.join("notes");
    let index_path = dir.join("index");
    let vec_db_path = dir.join("db");
    fs::create_dir_all(&notes_path).expect("Failed to create notes directory");
    fs::create_dir_all(&index_path).expect("Failed to create index directory");
    fs::create_dir_all(&vec_db_path).expect("Failed to create db directory");

    let db_path_str = dir.join(&vec_db_path);
    let db_path_str = db_path_str.to_str().unwrap();

    let db = async_db(db_path_str)
        .await
        .expect("Failed to connect to async db");
    db.call(|conn| {
        initialize_db(conn).expect("Failed to migrate db");
        Ok(())
    })
    .await
    .unwrap();

    index_dummy_notes_async(&db, dir.clone()).await;

    let app_config = AppConfig {
        notes_path: notes_path.display().to_string(),
        index_path: index_path.display().to_string(),
        vec_db_path: vec_db_path.to_str().unwrap().to_string(),
        storage_path: dir.display().to_string(),
        skills_path: String::from(""),
        deploy_key_path: String::from("test_deploy_key_path"),
        vapid_key_path: String::from("test_vapid_key_path"),
        note_search_api_url: String::from("http://localhost:2222"),
        gmail_api_client_id: String::from("test_client_id"),
        gmail_api_client_secret: String::from("test_client_secret"),
        google_search_api_key: String::from("test_google_search_key"),
        google_search_cx_id: String::from("test_cx_id"),
        openai_model: String::from("gpt-4o"),
        openai_api_hostname: String::from("https://api.openai.com"),
        openai_api_key: String::from("test-api-key"),
        system_message: String::from("You are a helpful assistant."),
    };
    let app_state = AppState::new(db, app_config, None);
    (app(Arc::new(RwLock::new(app_state.clone()))), app_state)
}

/// Creates a test skill in the given directory.
#[allow(dead_code)]
pub fn create_test_skill(base_dir: &Path, name: &str, description: &str) -> PathBuf {
    use std::fs;
    let skill_dir = base_dir.join(name);
    fs::create_dir_all(&skill_dir).expect("Failed to create skill directory");

    let skill_content = format!(
        r#"---
name: {}
description: {}
---

This is the body content of {}.
It contains instructions for using this skill.
"#,
        name, description, name
    );

    fs::write(skill_dir.join("SKILL.md"), skill_content).expect("Failed to write SKILL.md");

    skill_dir
}

/// Creates a test app with skills directory populated with test skills.
#[allow(dead_code)]
pub async fn test_app_with_skills() -> Router {
    use std::fs;

    // Create a unique directory for the test using a UUID to avoid
    // collisions between tests running in the same second
    let temp_dir = env::temp_dir();
    let dir = temp_dir.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("Failed to create base directory");

    // Create the required directories
    let notes_path = dir.join("notes");
    let index_path = dir.join("index");
    let vec_db_path = dir.join("db");
    let skills_path = dir.join("skills");
    fs::create_dir_all(&notes_path).expect("Failed to create notes directory");
    fs::create_dir_all(&index_path).expect("Failed to create index directory");
    fs::create_dir_all(&vec_db_path).expect("Failed to create db directory");
    fs::create_dir_all(&skills_path).expect("Failed to create skills directory");

    // Create test skills
    create_test_skill(&skills_path, "test-repo", "A test skill for repositories");
    create_test_skill(
        &skills_path,
        "pdf-processing",
        "Process and extract data from PDF files",
    );

    let db_path_str = dir.join("db");
    let db_path_str = db_path_str.to_str().unwrap();

    let db = async_db(db_path_str)
        .await
        .expect("Failed to connect to async db");
    db.call(|conn| {
        initialize_db(conn).expect("Failed to migrate db");
        Ok(())
    })
    .await
    .unwrap();

    index_dummy_notes_async(&db, dir.clone()).await;

    let app_config = AppConfig {
        notes_path: notes_path.display().to_string(),
        index_path: index_path.display().to_string(),
        vec_db_path: vec_db_path.to_str().unwrap().to_string(),
        storage_path: dir.display().to_string(),
        skills_path: skills_path.display().to_string(),
        deploy_key_path: String::from("test_deploy_key_path"),
        vapid_key_path: String::from("test_vapid_key_path"),
        note_search_api_url: String::from("http://localhost:2222"),
        gmail_api_client_id: String::from("test_client_id"),
        gmail_api_client_secret: String::from("test_client_secret"),
        google_search_api_key: String::from("test_google_search_key"),
        google_search_cx_id: String::from("test_cx_id"),
        openai_model: String::from("gpt-4o"),
        openai_api_hostname: String::from("https://api.openai.com"),
        openai_api_key: String::from("test-api-key"),
        system_message: String::from("You are a helpful assistant."),
    };
    let skill_registry = SkillRegistry::new(skills_path.display().to_string()).ok();
    let app_state = AppState::new(db, app_config, skill_registry);
    app(Arc::new(RwLock::new(app_state)))
}

/// Creates a chat message and indexes it for full-text search.
#[allow(dead_code)]
pub async fn create_and_index_chat_message(
    db: &tokio_rusqlite::Connection,
    storage_path: &str,
    session_id: &str,
    role: Role,
    content: &str,
) {
    // Create the session
    use hq::ai::chat::models::SessionMode;
    get_or_create_session(db, session_id, &[], SessionMode::Chat)
        .await
        .unwrap();

    // Create and insert the message
    let msg = Message::new(role, content);
    insert_chat_message(db, session_id, &msg).await.unwrap();

    // Index all chat sessions
    let index_path = format!("{}/index", storage_path);
    index_all_chat_sessions(db, &index_path).await.unwrap();
}
