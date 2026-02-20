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
}