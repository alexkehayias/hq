//! Integration tests for the skills API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serial_test::serial;
    use tower::util::ServiceExt;

    use crate::test_utils::{body_to_string, test_app, test_app_with_skills, create_test_skill};

    /// Helper: create a test skill with subdirectories (scripts/, references/).
    fn create_test_skill_with_files(base_dir: &Path, name: &str) {
        let skill_dir = base_dir.join(name);
        fs::create_dir_all(&skill_dir).expect("Failed to create skill directory");

        let skill_content = format!(
            r#"---
name: {}
description: A skill with subdirectories for testing.
---

This skill has scripts and references.
"#,
            name
        );
        fs::write(skill_dir.join("SKILL.md"), skill_content)
            .expect("Failed to write SKILL.md");

        // Create scripts subdirectory
        let scripts_dir = skill_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).expect("Failed to create scripts dir");
        fs::write(scripts_dir.join("process.py"), "#!/usr/bin/env python3\nprint('hello')")
            .expect("Failed to write process.py");
        fs::write(scripts_dir.join("run.sh"), "#!/bin/bash\necho 'running'")
            .expect("Failed to write run.sh");

        // Create references subdirectory
        let refs_dir = skill_dir.join("references");
        fs::create_dir_all(&refs_dir).expect("Failed to create references dir");
        fs::write(refs_dir.join("guide.md"), "# Guide\n\nReference guide.")
            .expect("Failed to write guide.md");
    }

    /// Tests listing all skills returns the correct count and names.
    #[tokio::test]
    #[serial]
    async fn it_lists_skills() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("test-repo"));
        assert!(body.contains("pdf-processing"));
        assert!(body.contains("A test skill for repositories"));
        assert!(body.contains("Process and extract data from PDF files"));
    }

    /// Tests listing skills returns empty list when no skills directory exists.
    #[tokio::test]
    #[serial]
    async fn it_returns_empty_skills_list() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"skills\":[]"));
    }

    /// Tests getting skill detail returns all frontmatter fields and body.
    #[tokio::test]
    #[serial]
    async fn it_gets_skill_detail() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("test-repo"));
        assert!(body.contains("A test skill for repositories"));
        assert!(body.contains("body"));
        assert!(body.contains("This is the body content of test-repo"));
    }

    /// Tests getting a non-existent skill returns 404.
    #[tokio::test]
    #[serial]
    async fn it_returns_404_for_unknown_skill() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/nonexistent-skill")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Tests listing files in a skill returns SKILL.md.
    #[tokio::test]
    #[serial]
    async fn it_lists_skill_files() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("SKILL.md"));
        assert!(body.contains("\"is_directory\":false"));
    }

    /// Tests listing files includes files from subdirectories.
    #[tokio::test]
    #[serial]
    async fn it_lists_skill_files_with_subdirectories() {
        use std::sync::{Arc, RwLock};

        use hq::api::{AppState, app};
        use hq::ai::skills::SkillRegistry;
        use hq::core::AppConfig;
        use hq::core::db::async_db;

        let temp_dir = std::env::temp_dir();
        let dir = temp_dir.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&dir).expect("Failed to create base directory");

        let skills_path = dir.join("skills");
        fs::create_dir_all(&skills_path).expect("Failed to create skills dir");

        create_test_skill_with_files(&skills_path, "full-skill");

        // Build a minimal app with just this skill
        let notes_path = dir.join("notes");
        let index_path = dir.join("index");
        let vec_db_path = dir.join("db");
        fs::create_dir_all(&notes_path).expect("Failed to create notes dir");
        fs::create_dir_all(&index_path).expect("Failed to create index dir");
        fs::create_dir_all(&vec_db_path).expect("Failed to create db dir");

        let db_path_str = dir.join("db").to_str().unwrap().to_string();
        let db = async_db(&db_path_str)
            .await
            .expect("Failed to connect to db");

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
        let skill_registry =
            SkillRegistry::new(skills_path.display().to_string()).unwrap();
        let app_state = AppState::new(db, app_config, skill_registry);
        let app = app(Arc::new(RwLock::new(app_state)));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/full-skill/files")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("SKILL.md"));
        assert!(body.contains("scripts/process.py"));
        assert!(body.contains("scripts/run.sh"));
        assert!(body.contains("references/guide.md"));
    }

    /// Tests reading a file from a skill.
    #[tokio::test]
    #[serial]
    async fn it_reads_skill_file() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files/SKILL.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("test-repo"));
        assert!(body.contains("A test skill for repositories"));
        assert!(body.contains("This is the body content of test-repo"));
    }

    /// Tests reading a non-existent file returns 404.
    #[tokio::test]
    #[serial]
    async fn it_returns_404_for_nonexistent_file() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files/nonexistent.py")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Tests writing to a file in a skill.
    #[tokio::test]
    #[serial]
    async fn it_writes_skill_file() {
        let app = test_app_with_skills().await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files/SKILL.md")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "content": "---\nname: test-repo\ndescription: A test skill for repositories\n---\n\nupdated content"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"success\":true"));

        // Verify the content was actually written
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files/SKILL.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("updated content"));
    }

    /// Tests creating a new file in a subdirectory.
    #[tokio::test]
    #[serial]
    async fn it_creates_file_in_subdirectory() {
        let app = test_app_with_skills().await;

        // Write a new file in a scripts subdirectory
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files/scripts/test.py")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "content": "print('hello')"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify the file was created and readable
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files/scripts/test.py")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("print('hello')"));
    }

    /// Tests path traversal is rejected.
    #[tokio::test]
    #[serial]
    async fn it_rejects_path_traversal() {
        let app = test_app_with_skills().await;

        // Attempt to write outside the skill directory
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/test-repo/files/../../outside.txt")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "content": "should not be written"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// Tests listing files returns 404 for unknown skill.
    #[tokio::test]
    #[serial]
    async fn it_returns_404_listing_files_for_unknown_skill() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/nonexistent/files")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Tests reading a file returns 404 for unknown skill.
    #[tokio::test]
    #[serial]
    async fn it_returns_404_reading_file_for_unknown_skill() {
        let app = test_app_with_skills().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/skills/nonexistent/files/SKILL.md")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Tests that skills created after the app started are visible via the API
    /// when the shared registry is reloaded.
    #[tokio::test]
    #[serial]
    async fn it_discovers_new_skills_after_registry_reload() {
        let (app, state) = crate::test_utils::test_app_with_state().await;

        // Verify initial empty state
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"skills\":[]"), "Expected empty skills: {}", body);

        // Create a new skill on disk (simulating save_skill)
        let skills_path = state.config.skills_path.clone();
        create_test_skill(
            std::path::Path::new(&skills_path),
            "new-skill",
            "A newly created skill",
        );

        // Reload the shared registry
        state.skill_registry.write().unwrap().reload().unwrap();

        // Verify the new skill is now visible via the API
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/skills")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_to_string(response.into_body()).await;
        assert!(
            body.contains("new-skill"),
            "Expected new-skill in response: {}",
            body
        );
    }
}
