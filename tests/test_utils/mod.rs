//! Test utilities for integration tests
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use axum::{Router, body::Body};

use hq::api::AppState;
use hq::api::app;
use hq::core::AppConfig;
use hq::core::db::async_db;
use hq::core::db::initialize_db;
use hq::search::index_all;

/// Creates a test application router with temporary directories.
///
/// Anything that uses this fixture can not be run in parallel due
/// to a lock held by `tantivy` during index writing so add a
/// `#[serial]` to the test function or run `cargo test --
/// --test-threads=1`.
pub async fn test_app() -> Router {
    // Create a unique directory for the test with a randomly
    // generated name using a timestamp to avoid collisions and
    // vulnerabilities
    let temp_dir = env::temp_dir();
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();
    let dir = temp_dir.join(ts);
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
    let app_state = AppState::new(db, app_config);
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
