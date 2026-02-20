//! Integration tests for the calendar API endpoints

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

    /// Tests calendar endpoint returns 400 when email is missing
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_email() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/calendar")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required param should return 400 Bad Request
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Tests calendar endpoint returns 500 when no refresh token exists
    #[tokio::test]
    #[serial]
    async fn it_returns_500_for_missing_refresh_token() {
        let app = test_app().await;

        // Request with email but no refresh token in DB
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/calendar?email=nonexistent@test.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 500 when no refresh token found for the email
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests calendar endpoint accepts days_ahead parameter
    #[tokio::test]
    #[serial]
    async fn it_accepts_days_ahead_parameter() {
        let app = test_app().await;

        // Request with days_ahead but no refresh token in DB
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/calendar?email=nonexistent@test.com&days_ahead=14")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Returns 500 because no refresh token, but accepts the param
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests calendar endpoint accepts calendar_id parameter
    #[tokio::test]
    #[serial]
    async fn it_accepts_calendar_id_parameter() {
        let app = test_app().await;

        // Request with calendar_id but no refresh token in DB
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/calendar?email=nonexistent@test.com&calendar_id=my-calendar")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Returns 500 because no refresh token, but accepts the param
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests calendar endpoint handles negative days_ahead
    #[tokio::test]
    #[serial]
    async fn it_handles_negative_days_ahead() {
        let app = test_app().await;

        // Request with negative days_ahead
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/calendar?email=nonexistent@test.com&days_ahead=-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Returns 500 because no refresh token, but accepts negative param
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}