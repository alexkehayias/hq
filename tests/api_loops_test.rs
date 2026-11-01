//! Integration tests for the loops API endpoint

mod test_utils;

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use serial_test::serial;
    use tower::util::ServiceExt;

    use crate::test_utils::{body_to_string, test_app, test_app_with_state};

    /// Tests that listing loops returns an empty collection initially.
    #[tokio::test]
    #[serial]
    async fn it_gets_empty_loops() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/loops")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"loops\""));
        assert!(body.contains("[]"));
    }

    /// Tests that a persisted loop is returned with its channels and status.
    #[tokio::test]
    #[serial]
    async fn it_lists_loops_with_channels() {
        let (app, state) = test_app_with_state().await;
        let db = state.db.clone();

        db.call(|conn| {
            conn.execute(
                "INSERT INTO channel (id, source) VALUES ('alpha', 'socket')",
                [],
            )?;
            conn.execute(
                "INSERT INTO channel (id, source) VALUES ('beta', 'server')",
                [],
            )?;
            conn.execute(
                "INSERT INTO loop (id, system_prompt, tools)
                 VALUES ('loop-1', 'You are a loop.', '[\"bash\"]')",
                [],
            )?;
            conn.execute(
                "INSERT INTO loop_channel (loop_id, channel_id, position)
                 VALUES ('loop-1', 'alpha', 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO loop_emit_channel (loop_id, channel_id)
                 VALUES ('loop-1', 'beta')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/loops")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"loop-1\""));
        assert!(body.contains("\"alpha\""));
        assert!(body.contains("\"emit_channels\":[\"beta\"]"), "{body}");
        assert!(body.contains("\"running\":false"));
    }

    /// Tests that a loop's system prompt can be replaced.
    #[tokio::test]
    #[serial]
    async fn it_updates_loop_system_prompt() {
        let (app, state) = test_app_with_state().await;
        let db = state.db.clone();

        db.call(|conn| {
            conn.execute(
                "INSERT INTO loop (id, system_prompt, tools)
                 VALUES ('loop-1', 'Old prompt.', '[\"bash\"]')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/loops/loop-1")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"system_prompt": "New prompt."}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_to_string(response.into_body()).await;
        assert!(body.contains("\"success\":true"), "{body}");

        let prompt: String = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT system_prompt FROM loop WHERE id = 'loop-1'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(prompt, "New prompt.");
    }

    /// Tests that updating a missing loop returns 404.
    #[tokio::test]
    #[serial]
    async fn it_returns_404_when_updating_missing_loop() {
        let app = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/loops/missing")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"system_prompt": "New prompt."}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Tests that a blank system prompt is rejected.
    #[tokio::test]
    #[serial]
    async fn it_rejects_blank_loop_system_prompt() {
        let (app, state) = test_app_with_state().await;
        let db = state.db.clone();

        db.call(|conn| {
            conn.execute(
                "INSERT INTO loop (id, system_prompt, tools)
                 VALUES ('loop-1', 'Old prompt.', '[\"bash\"]')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/loops/loop-1")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"system_prompt": "   "}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let prompt: String = db
            .call(|conn| {
                Ok(conn.query_row(
                    "SELECT system_prompt FROM loop WHERE id = 'loop-1'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(prompt, "Old prompt.");
    }
}
