//! Integration tests for the file uploads API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tokio_rusqlite::rusqlite;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serial_test::serial;
    use tower::util::ServiceExt;

    use crate::test_utils::{body_to_string, test_app_with_state};

    const BOUNDARY: &str = "----hqtestboundary";

    /// A minimal byte payload. Content is not sniffed by the endpoint, so any
    /// bytes work for exercising the upload path.
    const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nnot-a-real-png";

    /// Build a `multipart/form-data` body from `(field name, optional filename, data)` parts.
    fn multipart_body(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, filename, data) in parts {
            body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
            match filename {
                Some(fname) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{fname}\"\r\n\
                         Content-Type: application/octet-stream\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        body
    }

    fn upload_request(body: Vec<u8>) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/files")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from(body))
            .unwrap()
    }

    fn workspace_upload_request(body: Vec<u8>) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/files/workspace")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from(body))
            .unwrap()
    }

    fn delete_request(session_id: &str, filename: &str) -> Request<Body> {
        Request::builder()
            .method("DELETE")
            .uri(format!("/api/files/{session_id}/{filename}"))
            .body(Body::empty())
            .unwrap()
    }

    fn get_request(session_id: &str, filename: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(format!("/api/files/{session_id}/{filename}"))
            .body(Body::empty())
            .unwrap()
    }

    fn files_dir(storage: &str, session_id: &str) -> PathBuf {
        Path::new(storage)
            .join("workspace")
            .join(session_id)
            .join("files")
    }

    /// Uploading an image stores it under the session workspace `files/` dir and
    /// returns its paths.
    #[tokio::test]
    #[serial]
    async fn it_stores_uploaded_image() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-abc";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("screenshot.png"), PNG_BYTES),
        ]);

        let response = app.oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let stored = files_dir(&storage, session_id).join("screenshot.png");
        assert!(stored.exists(), "file should be written to the workspace");
        assert_eq!(std::fs::read(&stored).unwrap(), PNG_BYTES);

        let json: serde_json::Value =
            serde_json::from_str(&body_to_string(response.into_body()).await).unwrap();
        let file = &json["files"][0];
        assert_eq!(file["filename"], "screenshot.png");
        assert_eq!(file["path"], "files/screenshot.png");
        assert_eq!(file["sandbox_path"], "/files/screenshot.png");
        assert_eq!(file["content_type"], "image/png");
        assert_eq!(file["size"], PNG_BYTES.len());
    }

    /// An uploaded file can be fetched back with its content type.
    #[tokio::test]
    #[serial]
    async fn it_serves_uploaded_file() {
        let (app, _state) = test_app_with_state().await;
        let session_id = "session-serve";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("screenshot.png"), PNG_BYTES),
        ]);
        let response = app.clone().oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(get_request(session_id, "screenshot.png"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("content-type").unwrap(), "image/png");

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), PNG_BYTES);
    }

    /// Fetching a file that isn't there is a 404.
    #[tokio::test]
    #[serial]
    async fn it_returns_not_found_when_serving_missing_file() {
        let (app, _state) = test_app_with_state().await;

        let response = app
            .oneshot(get_request("session-missing", "nope.png"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Fetching a file with an unsupported extension is rejected.
    #[tokio::test]
    #[serial]
    async fn it_rejects_unsupported_type_on_serve() {
        let (app, _state) = test_app_with_state().await;

        let response = app
            .oneshot(get_request("session-serve-bad", "notes.txt"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// PDFs are not accepted; only images are.
    #[tokio::test]
    #[serial]
    async fn it_rejects_pdf_upload() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-pdf";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("doc.PDF"), b"%PDF-1.4 fake"),
        ]);

        let response = app.oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!files_dir(&storage, session_id).join("doc.PDF").exists());
    }

    /// Unsupported file types are rejected and nothing is written.
    #[tokio::test]
    #[serial]
    async fn it_rejects_unsupported_type() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-bad-type";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("notes.txt"), b"hello"),
        ]);

        let response = app.oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!files_dir(&storage, session_id).join("notes.txt").exists());
    }

    /// Text/code files uploaded from the workspace panel are stored in the
    /// same `files/` directory as image attachments.
    #[tokio::test]
    #[serial]
    async fn it_stores_uploaded_text_file() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-text";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("notes.md"), b"# hello\n"),
        ]);

        let response = app.oneshot(workspace_upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let stored = files_dir(&storage, session_id).join("notes.md");
        assert_eq!(std::fs::read_to_string(stored).unwrap(), "# hello\n");

        let json: serde_json::Value =
            serde_json::from_str(&body_to_string(response.into_body()).await).unwrap();
        let file = &json["files"][0];
        assert_eq!(file["filename"], "notes.md");
        assert_eq!(file["path"], "files/notes.md");
        assert_eq!(file["sandbox_path"], "/files/notes.md");
        assert_eq!(file["content_type"], "text/plain; charset=utf-8");
        assert_eq!(file["size"], 8);
    }

    /// Extensionless text files (README, Makefile) are accepted by the
    /// workspace upload endpoint.
    #[tokio::test]
    #[serial]
    async fn it_accepts_extensionless_text_file() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-text-no-ext";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("README"), b"hello\n"),
        ]);

        let response = app.oneshot(workspace_upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let stored = files_dir(&storage, session_id).join("README");
        assert_eq!(std::fs::read_to_string(stored).unwrap(), "hello\n");
    }

    /// Binary workspace uploads are rejected before anything is written.
    #[tokio::test]
    #[serial]
    async fn it_rejects_binary_workspace_upload() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-workspace-binary";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("blob.bin"), b"\x00\x01\x02"),
        ]);

        let response = app.oneshot(workspace_upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert!(!files_dir(&storage, session_id).join("blob.bin").exists());
    }

    /// Hidden filenames are rejected because the workspace listing hides
    /// dotfiles, so the upload would silently disappear from the panel.
    #[tokio::test]
    #[serial]
    async fn it_rejects_hidden_workspace_upload() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-workspace-hidden";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some(".env"), b"SECRET=1\n"),
        ]);

        let response = app.oneshot(workspace_upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!files_dir(&storage, session_id).join(".env").exists());
    }

    /// A second workspace upload with the same name is stored beside the first
    /// instead of overwriting it.
    #[tokio::test]
    #[serial]
    async fn it_renames_colliding_workspace_uploads() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-workspace-collision";

        for (content, expected_path) in [
            ("first\n", "files/notes.md"),
            ("second\n", "files/notes-1.md"),
        ] {
            let body = multipart_body(&[
                ("session_id", None, session_id.as_bytes()),
                ("file", Some("notes.md"), content.as_bytes()),
            ]);
            let response = app
                .clone()
                .oneshot(workspace_upload_request(body))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let json: serde_json::Value =
                serde_json::from_str(&body_to_string(response.into_body()).await).unwrap();
            assert_eq!(json["files"][0]["path"], expected_path);
        }

        let dir = files_dir(&storage, session_id);
        assert_eq!(
            std::fs::read_to_string(dir.join("notes.md")).unwrap(),
            "first\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("notes-1.md")).unwrap(),
            "second\n"
        );
    }

    /// Directory components in the filename are stripped so the upload cannot
    /// escape the `files/` directory.
    #[tokio::test]
    #[serial]
    async fn it_sanitizes_traversal_filename() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-traversal";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("../../evil.png"), PNG_BYTES),
        ]);

        let response = app.oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Stored as a plain basename inside the session's files dir.
        let stored = files_dir(&storage, session_id).join("evil.png");
        assert!(stored.exists());
        // Nothing escaped to the storage root.
        assert!(!Path::new(&storage).join("evil.png").exists());
        assert!(
            !Path::new(&storage)
                .join("workspace")
                .join("evil.png")
                .exists()
        );
    }

    /// A missing `session_id` field is a client error.
    #[tokio::test]
    #[serial]
    async fn it_rejects_missing_session_id() {
        let (app, _state) = test_app_with_state().await;

        let body = multipart_body(&[("file", Some("screenshot.png"), PNG_BYTES)]);
        let response = app.oneshot(upload_request(body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// A `session_id` containing path separators is rejected.
    #[tokio::test]
    #[serial]
    async fn it_rejects_traversal_session_id() {
        let (app, _state) = test_app_with_state().await;

        let body = multipart_body(&[
            ("session_id", None, b"../escape"),
            ("file", Some("screenshot.png"), PNG_BYTES),
        ]);
        let response = app.oneshot(upload_request(body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// A request with no file field is a client error.
    #[tokio::test]
    #[serial]
    async fn it_rejects_missing_file() {
        let (app, _state) = test_app_with_state().await;

        let body = multipart_body(&[("session_id", None, b"session-no-file")]);
        let response = app.oneshot(upload_request(body)).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Deleting an uploaded file removes it from the session workspace.
    #[tokio::test]
    #[serial]
    async fn it_deletes_uploaded_file() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-delete";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("screenshot.png"), PNG_BYTES),
        ]);
        let response = app.clone().oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let stored = files_dir(&storage, session_id).join("screenshot.png");
        assert!(stored.exists());

        let response = app
            .oneshot(delete_request(session_id, "screenshot.png"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(
            !stored.exists(),
            "file should be removed from the workspace"
        );
    }

    /// Deleting a file that isn't there is a 404.
    #[tokio::test]
    #[serial]
    async fn it_returns_not_found_when_deleting_missing_file() {
        let (app, _state) = test_app_with_state().await;

        let response = app
            .oneshot(delete_request("session-missing", "nope.png"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// A delete is scoped to the session in the path: another session's file
    /// with the same name is left untouched.
    #[tokio::test]
    #[serial]
    async fn it_scopes_delete_to_session() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let owner = "session-owner";

        let body = multipart_body(&[
            ("session_id", None, owner.as_bytes()),
            ("file", Some("screenshot.png"), PNG_BYTES),
        ]);
        let response = app.clone().oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(delete_request("session-other", "screenshot.png"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let stored = files_dir(&storage, owner).join("screenshot.png");
        assert!(stored.exists(), "owner's file must not be deleted");
    }

    /// A session ID that isn't safe as a path segment is rejected.
    #[tokio::test]
    #[serial]
    async fn it_rejects_invalid_session_id_on_delete() {
        let (app, _state) = test_app_with_state().await;

        let response = app
            .oneshot(delete_request("session.bad", "screenshot.png"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Files uploaded to a loop run's transcript land in the loop's shared
    /// workspace, so the run's bash/python tools (which mount that workspace)
    /// can see them.
    #[tokio::test]
    #[serial]
    async fn it_stores_loop_run_uploads_in_the_loop_workspace() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let db = state.db.clone();

        db.call(|conn| {
            conn.execute(
                "INSERT INTO channel (id, source) VALUES ('alpha', 'socket')",
                [],
            )?;
            conn.execute(
                "INSERT INTO loop (id, system_prompt, tools)
                 VALUES ('loop-1', 'You are a loop.', '[\"bash\"]')",
                [],
            )?;
            conn.execute(
                "INSERT INTO loop_run (id, loop_id, channel_id, event, status, session_id)
                 VALUES ('run-1', 'loop-1', 'alpha', 'event', 'success', 'run-1')",
                [],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .await
        .unwrap();

        let body = multipart_body(&[
            ("session_id", None, b"run-1"),
            ("file", Some("screenshot.png"), PNG_BYTES),
        ]);
        let response = app.clone().oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let stored = files_dir(&storage, "loop-1").join("screenshot.png");
        assert!(
            stored.exists(),
            "upload should be written to the loop's workspace"
        );
        assert!(
            !files_dir(&storage, "run-1").exists(),
            "no per-run workspace should be created"
        );

        // The run's transcript path serves the loop's file back.
        let response = app
            .clone()
            .oneshot(get_request("run-1", "screenshot.png"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), PNG_BYTES);

        let response = app
            .oneshot(delete_request("run-1", "screenshot.png"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(!stored.exists(), "delete should remove the loop's file");
    }
}
