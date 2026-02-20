//! Integration tests for the webhook API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serial_test::serial;
    use tower::util::ServiceExt;

    use crate::test_utils::test_app;

    /// Tests blurt webhook accepts valid notification
    #[tokio::test]
    #[serial]
    async fn it_accepts_valid_blurt_notification() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": 12345,
                            "title": "Test Notification",
                            "subtitle": Some("Subtitle"),
                            "body": "This is a test notification body",
                            "date": 1704067200,
                            "bundle_id": Some("com.example.app"),
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests blurt webhook returns 400 for missing required field (id)
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_id() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "Test Notification",
                            "body": "This is a test notification body",
                            "date": 1704067200,
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

    /// Tests blurt webhook returns 400 for missing required field (title)
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_title() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": 12345,
                            "body": "This is a test notification body",
                            "date": 1704067200,
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

    /// Tests blurt webhook returns 400 for missing required field (body)
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_body() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": 12345,
                            "title": "Test Notification",
                            "date": 1704067200,
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

    /// Tests blurt webhook returns 400 for missing required field (date)
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_date() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": 12345,
                            "title": "Test Notification",
                            "body": "This is a test notification body",
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

    /// Tests blurt webhook accepts notification without optional fields
    #[tokio::test]
    #[serial]
    async fn it_accepts_notification_without_optional_fields() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": 12345,
                            "title": "Test Notification",
                            "body": "This is a test notification body",
                            "date": 1704067200,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests blurt webhook accepts notification with null optional fields
    #[tokio::test]
    #[serial]
    async fn it_accepts_notification_with_null_optional_fields() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": 12345,
                            "title": "Test Notification",
                            "subtitle": null,
                            "body": "This is a test notification body",
                            "date": 1704067200,
                            "bundle_id": null,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests blurt webhook returns 400 for invalid JSON
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_invalid_json() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from("{invalid json}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Invalid JSON should return 400 Bad Request
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Tests blunt webhook returns 405 for GET request
    #[tokio::test]
    #[serial]
    async fn it_returns_405_for_get_request() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/webhook/blurt")
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Method not allowed for GET on POST endpoint
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}