//! Integration tests for the email API endpoints

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

    /// Tests the email unread endpoint returns 400 when email is missing
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_email_param() {
        let app = test_app().await;

        // Request without email query param should return error
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/email/unread")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required parameter should return 400 Bad Request
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Tests the email unread endpoint returns 500 when no refresh token exists
    #[tokio::test]
    #[serial]
    async fn it_returns_500_for_missing_refresh_token() {
        let app = test_app().await;

        // Request with email but no refresh token in DB
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/email/unread?email=nonexistent@test.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 500 when no refresh token found for the email
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests the email unread endpoint accepts limit parameter
    #[tokio::test]
    #[serial]
    async fn it_accepts_limit_parameter() {
        let app = test_app().await;

        // Request with email and limit param but no refresh token in DB
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/email/unread?email=nonexistent@test.com&limit=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should still return 500 because no refresh token, but it accepts the param
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests the email unread endpoint handles negative limit gracefully
    #[tokio::test]
    #[serial]
    async fn it_handles_negative_limit() {
        let app = test_app().await;

        // Request with negative limit value
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/email/unread?email=test@test.com&limit=-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Negative limit results in a server error (not 400) since it's parsed as i64
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
