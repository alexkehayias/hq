//! Embedded web assets, compiled into the binary when built with
//! `--features embed-assets` (prod). Dev builds serve `web-ui/src` from disk.

use axum::Router;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use http::{HeaderValue, StatusCode, header};
use include_dir::{Dir, include_dir};

static ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/web-ui/src");

/// Resolve a request path to its (content-type, bytes). Mirrors ServeDir's
/// directory-index behavior: a path that maps to a directory serves `index.html`.
fn resolve(path: &str) -> Option<(String, &'static [u8])> {
    let trimmed = path.trim_start_matches('/');

    let candidates: Vec<String> = if trimmed.is_empty() {
        vec!["index.html".to_string()]
    } else if trimmed.ends_with('/') {
        vec![format!("{trimmed}index.html")]
    } else {
        vec![trimmed.to_string(), format!("{trimmed}/index.html")]
    };

    for rel in candidates {
        if let Some(file) = ASSETS.get_file(&rel) {
            let content_type = mime_guess::from_path(&rel)
                .first_or_octet_stream()
                .as_ref()
                .to_owned();
            return Some((content_type, file.contents()));
        }
    }
    None
}

pub async fn handler(request: Request) -> Response {
    match resolve(request.uri().path()) {
        Some((content_type, bytes)) => (
            [
                (header::CONTENT_TYPE, HeaderValue::from_str(&content_type).unwrap()),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            ],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Attach the embedded-assets fallback to `router`.
pub fn attach_assets<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.fallback(handler)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request};
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new().fallback(handler)
    }

    async fn get(uri: &str) -> axum::response::Response {
        app()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn serves_index_at_root() {
        let res = get("/").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[tokio::test]
    async fn serves_directory_index_with_and_without_trailing_slash() {
        for uri in ["/chat", "/chat/"] {
            let res = get(uri).await;
            assert_eq!(res.status(), StatusCode::OK, "uri {uri}");
            assert_eq!(res.headers()[header::CONTENT_TYPE], "text/html");
        }
    }

    #[tokio::test]
    async fn serves_component_js_with_js_content_type() {
        let res = get("/components/hq-button.js").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/javascript");
    }

    #[tokio::test]
    async fn missing_path_returns_404() {
        let res = get("/nope/not-here.js").await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
