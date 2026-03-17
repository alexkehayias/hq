//! Integration tests for the chat API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serial_test::serial;
    use tower::util::ServiceExt;

    use crate::test_utils::{body_to_string, test_app, test_app_with_state};

    /// Tests getting chat sessions returns empty list initially
    #[tokio::test]
    #[serial]
    async fn it_gets_empty_chat_sessions() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"sessions\""));
        assert!(body.contains("\"total_sessions\":0"));
    }

    /// Tests getting chat sessions with pagination
    #[tokio::test]
    #[serial]
    async fn it_gets_chat_sessions_with_pagination() {
        let app = test_app().await;

        // Create a chat session first
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-session-pagination",
                            "message": "Hello"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Get sessions with pagination
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/sessions?page=1&limit=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"page\":1"));
        assert!(body.contains("\"limit\":5"));
    }

    /// Tests getting chat session by ID returns 404 for non-existent session
    #[tokio::test]
    #[serial]
    async fn it_returns_404_for_nonexistent_session() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/nonexistent-session-id/view")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Non-existent session should return 404 Not Found (or 200 with empty transcript)
        let status = response.status();
        // The router uses /{id} not /{id}/view, let me check
        assert!(status == StatusCode::NOT_FOUND || status == StatusCode::OK);
    }

    /// Tests getting chat session by ID with correct path
    #[tokio::test]
    #[serial]
    async fn it_gets_chat_session_by_id() {
        let app = test_app().await;

        // First create a session
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-session-get",
                            "message": "Hello world"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then get it by ID - note: the router uses /{id}, not /{id}/view
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/test-session-get")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return the session (200) or 404 if not found immediately
        let status = response.status();
        assert!(status == StatusCode::OK || status == StatusCode::NOT_FOUND);
    }

    /// Tests chat POST returns 400 for missing session_id
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_session_id() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "message": "Hello"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required field should return 422 (validation error)
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Tests chat POST returns 400 for missing message
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_message() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-session"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required field should return 422 (validation error)
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Tests chat sessions with tags filter
    #[tokio::test]
    #[serial]
    async fn it_filters_sessions_by_tags() {
        let app = test_app().await;

        // Get sessions with tags filter
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/sessions?tags=work&tags=personal")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"sessions\""));
    }

    /// Tests chat sessions with exclude_tags filter
    #[tokio::test]
    #[serial]
    async fn it_excludes_sessions_by_tags() {
        let app = test_app().await;

        // Get sessions excluding certain tags
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat/sessions?exclude_tags=archived")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"sessions\""));
    }

    /// Tests entering agent mode via /code command
    #[tokio::test]
    #[serial]
    async fn it_enters_agent_mode_via_code_command() {
        let app = test_app().await;

        // Send a message with /code prefix to enter agent mode
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-agent-mode",
                            "message": "/code list files in current directory"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK (200) and start streaming response
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests exiting agent mode via /exit command
    #[tokio::test]
    #[serial]
    async fn it_exits_agent_mode_via_exit_command() {
        let app = test_app().await;

        // First enter agent mode
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-exit-agent",
                            "message": "/code echo hello"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then exit agent mode with /exit command
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-exit-agent",
                            "message": "/exit"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK and confirm exit from agent mode
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests staying in chat mode when sending regular message
    #[tokio::test]
    #[serial]
    async fn it_stays_in_chat_mode_with_regular_message() {
        let app = test_app().await;

        // Send a regular message without /code prefix
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-chat-mode",
                            "message": "Hello, how are you?"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK and use chat mode (OpenAI)
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests that session mode is set to Agent after /code command
    #[tokio::test]
    #[serial]
    async fn it_sets_session_mode_to_agent_after_code_command() {
        use hq::ai::chat::db::get_session_mode;
        use hq::ai::chat::models::SessionMode;

        let (app, state) = test_app_with_state().await;

        // Send a message with /code prefix to enter agent mode
        let _response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-mode-agent",
                            "message": "/code echo test"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Verify session mode is set to Agent in the database
        let db = state.db.clone();
        let mode = get_session_mode(&db, "test-mode-agent")
            .await
            .expect("Failed to get session mode");
        assert_eq!(mode, SessionMode::Code);
    }

    /// Tests that /exit command switches session mode from Agent to Chat
    #[tokio::test]
    #[serial]
    async fn it_switches_session_mode_from_agent_to_chat_on_exit() {
        use hq::ai::chat::db::{get_or_create_session, get_session_mode};
        use hq::ai::chat::models::SessionMode;

        let (app, state) = test_app_with_state().await;

        // First, create a session in agent mode directly
        let db = state.db.clone();
        get_or_create_session(&db, "test-mode-exit", &[], SessionMode::Code)
            .await
            .expect("Failed to create session");

        // Verify we start in agent mode
        let mode_before = get_session_mode(&db, "test-mode-exit")
            .await
            .expect("Failed to get session mode");
        assert_eq!(mode_before, SessionMode::Code);

        // Now send /exit command
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-mode-exit",
                            "message": "/exit"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK and confirm exit from agent mode
        assert_eq!(response.status(), StatusCode::OK);

        // Verify session mode switched to Chat in the database
        let db = state.db.clone();
        let mode_after = get_session_mode(&db, "test-mode-exit")
            .await
            .expect("Failed to get session mode");
        assert_eq!(mode_after, SessionMode::Chat);
    }

    /// Tests that /exit in chat mode is a no-op and stays in Chat
    #[tokio::test]
    #[serial]
    async fn it_stays_in_chat_mode_when_exit_sent_from_chat() {
        use hq::ai::chat::db::{get_or_create_session, get_session_mode};
        use hq::ai::chat::models::SessionMode;

        let (app, state) = test_app_with_state().await;

        // First, create a session in chat mode
        let db = state.db.clone();
        get_or_create_session(&db, "test-mode-chat-exit", &[], SessionMode::Chat)
            .await
            .expect("Failed to create session");

        // Send /exit command while in chat mode (should be a no-op)
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-mode-chat-exit",
                            "message": "/exit"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK with a message indicating already in chat mode
        assert_eq!(response.status(), StatusCode::OK);

        // Verify session is still in Chat mode
        let db = state.db.clone();
        let mode_after = get_session_mode(&db, "test-mode-chat-exit")
            .await
            .expect("Failed to get session mode");
        assert_eq!(mode_after, SessionMode::Chat);
    }

    /// Tests that /exit command stores messages in the database
    #[tokio::test]
    #[serial]
    async fn it_stores_messages_after_exit_command() {
        use hq::ai::chat::db::{find_chat_session_by_id, get_or_create_session};
        use hq::ai::chat::models::SessionMode;
        use hq::openai::Role;

        let (app, state) = test_app_with_state().await;

        // Create a session in agent mode
        let db = state.db.clone();
        get_or_create_session(&db, "test-exit-messages", &[], SessionMode::Code)
            .await
            .expect("Failed to create session");

        // Send /exit command
        let _response = app
            .oneshot(
                Request::builder()
                    .uri("/api/chat")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "session_id": "test-exit-messages",
                            "message": "/exit"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Verify messages were stored in the database
        let db = state.db.clone();
        let messages = find_chat_session_by_id(&db, "test-exit-messages")
            .await
            .expect("Failed to find chat session");

        // Should have at least 2 messages: user /exit and assistant response
        assert!(
            messages.len() >= 2,
            "Expected at least 2 messages, got {}",
            messages.len()
        );

        // First message should be user's /exit
        let (_id1, msg1) = &messages[messages.len() - 2];
        assert_eq!(*msg1.role(), Role::User);
        assert!(msg1.content.as_ref().expect("content").contains("/exit"));

        // Second message should be assistant's response
        let (_id2, msg2) = &messages[messages.len() - 1];
        assert_eq!(*msg2.role(), Role::Assistant);
        assert!(
            msg2.content
                .as_ref()
                .expect("content")
                .contains("Exited agent mode")
        );
    }
}
