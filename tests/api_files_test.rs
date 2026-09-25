//! Integration tests for the file uploads API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

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

    /// A PDF upload is accepted too.
    #[tokio::test]
    #[serial]
    async fn it_stores_uploaded_pdf() {
        let (app, state) = test_app_with_state().await;
        let storage = state.config.storage_path.clone();
        let session_id = "session-pdf";

        let body = multipart_body(&[
            ("session_id", None, session_id.as_bytes()),
            ("file", Some("doc.PDF"), b"%PDF-1.4 fake"),
        ]);

        let response = app.oneshot(upload_request(body)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let json: serde_json::Value =
            serde_json::from_str(&body_to_string(response.into_body()).await).unwrap();
        assert_eq!(json["files"][0]["content_type"], "application/pdf");
        assert!(files_dir(&storage, session_id).join("doc.PDF").exists());
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
}
