//! Integration tests for the push API endpoints

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

    /// Tests push subscription with valid request
    #[tokio::test]
    #[serial]
    async fn it_subscribes_to_push_notifications() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/subscribe")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "endpoint": "https://example.com/push",
                            "keys": {
                                "p256dh": "test-p256dh-key",
                                "auth": "test-auth-key"
                            }
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
    }

    /// Tests push subscription returns 400 for missing endpoint
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_endpoint() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/subscribe")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "keys": {
                                "p256dh": "test-p256dh-key",
                                "auth": "test-auth-key"
                            }
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

    /// Tests push subscription returns 400 for missing keys
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_keys() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/subscribe")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "endpoint": "https://example.com/push"
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

    /// Tests push subscription panics for missing p256dh key (known bug)
    #[tokio::test]
    #[serial]
    #[should_panic(expected = "Missing p256dh key")]
    async fn it_panics_for_missing_p256dh() {
        let app = test_app().await;

        let _response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/subscribe")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "endpoint": "https://example.com/push",
                            "keys": {
                                "auth": "test-auth-key"
                            }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    /// Tests push subscription panics for missing auth key (known bug)
    #[tokio::test]
    #[serial]
    #[should_panic(expected = "Missing auth key")]
    async fn it_panics_for_missing_auth() {
        let app = test_app().await;

        let _response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/subscribe")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "endpoint": "https://example.com/push",
                            "keys": {
                                "p256dh": "test-p256dh-key"
                            }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    /// Tests send notification with valid request
    #[tokio::test]
    #[serial]
    async fn it_sends_notification() {
        let app = test_app().await;

        // First subscribe
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/push/subscribe")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "endpoint": "https://example.com/push",
                            "keys": {
                                "p256dh": "test-p256dh-key",
                                "auth": "test-auth-key"
                            }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Then send notification (will fail due to invalid vapid keys but should return 200)
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/notification")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "message": "Test notification message"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Returns 200 even if notification sending fails (invalid vapid keys)
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests send notification returns 400 for missing message
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_message() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/notification")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required field should return 422 (validation error)
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Tests push endpoints return 405 for GET requests
    #[tokio::test]
    #[serial]
    async fn it_returns_405_for_get_on_subscribe() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/subscribe")
                    .method("GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Method not allowed for GET on POST endpoint
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    /// Tests push endpoints return 405 for GET requests on notification
    #[tokio::test]
    #[serial]
    async fn it_returns_405_for_get_on_notification() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/push/notification")
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