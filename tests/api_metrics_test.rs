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

    use crate::test_utils::{body_to_string, test_app};

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
                            "input": 100,
                            "output": 50,
                            "cache_read": 200,
                            "cache_write": 80
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Tests that a payload with optional `reasoning` is accepted
    #[tokio::test]
    #[serial]
    async fn it_records_metric_with_reasoning() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "input": 100,
                            "output": 50,
                            "cache_read": 200,
                            "cache_write": 80,
                            "reasoning": 40
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
                            "input": 100,
                            "output": 50,
                            "cache_read": 200,
                            "cache_write": 80
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
        // The recorded buckets should appear in the aggregated events
        assert!(body.contains("\"input\""));
        assert!(body.contains("\"output\""));
        assert!(body.contains("\"cache_read\""));
        assert!(body.contains("\"cache_write\""));
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
                            "input": 100,
                            "output": 50,
                            "cache_read": 200,
                            "cache_write": 80
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

    /// Tests getting the sessions metric counts sessions created per day
    #[tokio::test]
    #[serial]
    async fn it_gets_session_metrics() {
        let app = test_app().await;

        // Create two distinct chat sessions
        for session_id in ["sess-one", "sess-two"] {
            let _response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/chat")
                        .method("POST")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "session_id": session_id,
                                "message": "Hello"
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        // Get the sessions metric
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics?metric=sessions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"events\""));
        // Session counts are returned in the generic `value` field
        assert!(body.contains("\"value\""));
    }

    /// Tests that a payload missing all required bucket fields returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_missing_required_field() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing all required bucket fields should return 422 (serde deserialization error)
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Tests that a payload with a negative bucket value returns 422
    #[tokio::test]
    #[serial]
    async fn it_returns_422_for_negative_value() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "input": -1,
                            "output": 50,
                            "cache_read": 200,
                            "cache_write": 80
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // u32 rejects negative integers — serde deserialization fails with 422
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}