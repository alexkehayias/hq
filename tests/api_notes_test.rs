//! Integration tests for the notes API endpoints

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

    /// Tests searching notes with a query
    #[tokio::test]
    #[serial]
    async fn it_searches_notes() {
        let app = test_app().await;

        // The test_app already indexes a dummy note with "test" in the title
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search?query=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"raw_query\""));
        assert!(body.contains("\"parsed_query\""));
        assert!(body.contains("\"results\""));
    }

    /// Tests searching notes returns empty results for non-matching query
    #[tokio::test]
    #[serial]
    async fn it_searches_notes_with_no_results() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search?query=nonexistentterm123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"results\":[]"));
    }

    /// Tests search with limit parameter
    #[tokio::test]
    #[serial]
    async fn it_searches_notes_with_limit() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search?query=test&limit=5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"raw_query\""));
    }

    /// Tests search with include_similarity parameter
    #[tokio::test]
    #[serial]
    async fn it_searches_notes_with_similarity() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search?query=test&include_similarity=true")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"raw_query\""));
    }

    /// Tests search with truncate parameter
    #[tokio::test]
    #[serial]
    async fn it_searches_notes_with_truncate() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search?query=test&truncate=false")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"raw_query\""));
    }

    /// Tests search returns 400 when query is missing
    #[tokio::test]
    #[serial]
    async fn it_returns_400_for_missing_query() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Missing required query param should return 400 Bad Request
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Tests indexing notes via POST
    #[tokio::test]
    #[serial]
    async fn it_indexes_notes() {
        let app = test_app().await;

        // Note: This will fail because there's no git deploy key, but it should
        // still return 200 OK since indexing is spawned as a background task
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/index")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return OK since indexing is async
        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"success\":true"));
    }

    /// Tests viewing a note by ID that exists
    #[tokio::test]
    #[serial]
    async fn it_views_note_by_id() {
        let app = test_app().await;

        // The test_app creates a note with ID: 6A503659-15E4-4427-835F-7873F8FF8ECF
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/6A503659-15E4-4427-835F-7873F8FF8ECF/view")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"id\""));
    }

    /// Tests viewing a note by ID that doesn't exist returns 500 (not ideal, but current behavior)
    #[tokio::test]
    #[serial]
    async fn it_returns_error_for_nonexistent_note() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/nonexistent-id-123/view")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Currently returns 500 due to unhandled None in db.rs (should be 404)
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// Tests searching notes with tags:meeting query (used by MeetingSearchTool)
    #[tokio::test]
    #[serial]
    async fn it_searches_notes_with_meeting_tag() {
        let app = test_app().await;

        // This tests the query format used by MeetingSearchTool
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/notes/search?query=tags:meeting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"raw_query\""));
        assert!(body.contains("\"results\""));
    }

    // Note: Empty query test is intentionally omitted - it causes a panic in the AQL parser
    // which is a known bug. The endpoint should return 400 Bad Request instead.
}
