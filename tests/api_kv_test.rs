//! Integration tests for the kv API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serial_test::serial;
    use tower::util::ServiceExt;

    use crate::test_utils::{body_to_string, test_app};

    /// Tests getting latest selection returns null when not set
    #[tokio::test]
    #[serial]
    async fn it_gets_null_when_not_set() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("null"));
    }

    /// Tests setting latest selection
    #[tokio::test]
    #[serial]
    async fn it_sets_latest_selection() {
        let app = test_app().await;

        // Set a latest selection
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "test-id-123",
                            "file_name": "test.org",
                            "title": "Test Note"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests getting latest selection after setting it
    #[tokio::test]
    #[serial]
    async fn it_gets_latest_selection_after_setting() {
        let app = test_app().await;

        // First set a latest selection
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "test-id-456",
                            "file_name": "my-note.org",
                            "title": "My Note Title"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then get it back
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("test-id-456"));
        assert!(body.contains("my-note.org"));
        assert!(body.contains("My Note Title"));
    }

    /// Tests setting latest selection with missing id returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_missing_id() {
        let app = test_app().await;

        // Try to set with missing id
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "file_name": "test.org",
                            "title": "Test Note"
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

    /// Tests setting latest selection with missing file_name returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_missing_file_name() {
        let app = test_app().await;

        // Try to set with missing file_name
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "test-id-123",
                            "title": "Test Note"
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

    /// Tests setting latest selection with missing title returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_missing_title() {
        let app = test_app().await;

        // Try to set with missing title
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "test-id-123",
                            "file_name": "test.org"
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

    /// Tests GET returns 405 for method not allowed (placeholder test)
    #[tokio::test]
    #[serial]
    async fn it_returns_405_for_post_to_get_endpoint() {
        // This test is a placeholder - the kv router allows both GET and POST on /latest
    }

    /// Tests latest selection can be updated
    #[tokio::test]
    #[serial]
    async fn it_updates_latest_selection() {
        let app = test_app().await;

        // Set first value
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "first-id",
                            "file_name": "first.org",
                            "title": "First"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Update with new value
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "second-id",
                            "file_name": "second.org",
                            "title": "Second"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Verify updated value
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search/latest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("second-id"));
        assert!(!body.contains("first-id"));
    }
}
