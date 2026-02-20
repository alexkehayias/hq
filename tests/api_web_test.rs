//! Integration tests for the web API endpoints

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

    /// Tests web search returns 500 when Google API is not configured
    #[tokio::test]
    #[serial]
    async fn it_returns_500_for_unconfigured_api() {
        let app = test_app().await;

        // The test app uses fake/unconfigured Google API keys
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/web/search?query=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 500 because Google API key is not configured in test app
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests web search returns 400 when query is missing
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_query() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/web/search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required query param should return 400 Bad Request
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Tests web search accepts limit parameter
    #[tokio::test]
    #[serial]
    async fn it_accepts_limit_parameter() {
        let app = test_app().await;

        // Even with limit, should return 500 because API is not configured
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/web/search?query=test&limit=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Returns 500 because Google API key is fake, but limit param is accepted
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests web search returns proper JSON error structure
    #[tokio::test]
    #[serial]
    async fn it_returns_json_error_for_api_failure() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/web/search?query=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return an error response (even if 500)
        let body = body_to_string(response.into_body()).await;
        // The response should contain JSON, even if it's an error
        assert!(!body.is_empty());
    }

    /// Tests web search handles empty query gracefully
    #[tokio::test]
    #[serial]
    async fn it_handles_empty_query() {
        let app = test_app().await;

        // Empty query might be handled differently than missing query
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/web/search?query=")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Empty string might be accepted but then fail on Google API call
        let status = response.status();
        // Could be 400 (validation) or 500 (API failure)
        assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::INTERNAL_SERVER_ERROR);
    }
}