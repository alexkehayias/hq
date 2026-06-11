//! Integration tests for the metrics API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serial_test::serial;
    use tower::util::ServiceExt;

    use hq::ai::chat::db::get_or_create_session;
    use hq::ai::chat::models::SessionMode;

    use crate::test_utils::{body_to_string, test_app, test_app_with_state};

    /// Tests recording a metric via POST
    #[tokio::test]
    #[serial]
    async fn it_records_metric() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "token-count",
                            "value": 20,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests getting metrics returns empty array initially
    #[tokio::test]
    #[serial]
    async fn it_gets_empty_metrics() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"events\""));
    }

    /// Tests getting metrics after recording one
    #[tokio::test]
    #[serial]
    async fn it_gets_recorded_metrics() {
        let app = test_app().await;

        // First record a metric
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "token-count",
                            "value": 100,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Then get metrics
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"events\""));
        // The recorded metric should appear in the events
        assert!(body.contains("token-count"));
    }

    /// Tests getting metrics with limit_days parameter
    #[tokio::test]
    #[serial]
    async fn it_gets_metrics_with_limit_days() {
        let app = test_app().await;

        // First record a metric
        let _response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "token-count",
                            "value": 50,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Get metrics with limit_days=7
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics?limit_days=7")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"events\""));
    }

    /// Tests that recording a metric with invalid name returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_invalid_metric_name() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "invalid-metric",
                            "value": 20,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Invalid metric name should return 422 Unprocessable Entity (validation error)
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Tests that recording a metric with missing value returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_missing_value() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "token-count",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required field should return 422 Unprocessable Entity (validation error)
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Tests that recording a metric with missing name returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_missing_name() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "value": 20,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required field should return 422 Unprocessable Entity (validation error)
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Tests recording a session-count metric via the API
    #[tokio::test]
    #[serial]
    async fn it_records_session_count_via_api() {
        let app = test_app().await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "session-count",
                            "value": 1,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        // Verify it appears in the metrics response
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("session-count"));
    }

    /// Tests that creating a new session records a session-count metric
    #[tokio::test]
    #[serial]
    async fn it_records_session_count_on_new_session() {
        let (app, state) = test_app_with_state().await;
        let db = state.db.clone();

        // Create a new session
        get_or_create_session(&db, "test-session-metric-1", &[], SessionMode::Chat)
            .await
            .unwrap();

        // Verify the session-count metric was recorded
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("session-count"));
    }

    /// Tests that creating the same session twice does not duplicate the metric
    #[tokio::test]
    #[serial]
    async fn it_does_not_duplicate_session_count_on_existing_session() {
        let (app, state) = test_app_with_state().await;
        let db = state.db.clone();

        // Create a new session
        get_or_create_session(&db, "test-session-metric-2", &[], SessionMode::Chat)
            .await
            .unwrap();

        // Create the same session again (should be a no-op for the metric)
        get_or_create_session(&db, "test-session-metric-2", &[], SessionMode::Chat)
            .await
            .unwrap();

        // Verify only one session-count metric event was recorded
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("session-count"));

        // Parse the response to verify only one event with value 1
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let events = parsed["events"].as_array().unwrap();
        let session_events: Vec<&serde_json::Value> = events
            .iter()
            .filter(|e| e["name"] == "session-count")
            .collect();
        assert_eq!(session_events.len(), 1);
        assert_eq!(session_events[0]["value"], 1);
    }
}
