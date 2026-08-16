//! Serve the web UI from disk (dev), so changes appear without a rebuild.

use axum::Router;
use axum::middleware;
use axum::extract::Request;
use axum::response::Response;
use http::{HeaderValue, header};
use tower::ServiceBuilder;
use tower_http::services::ServeDir;

/// Attach the disk-backed static-asset fallback to `router`.
pub fn attach_assets<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.fallback_service(
        ServiceBuilder::new()
            .layer(middleware::from_fn(set_static_cache_control))
            .service(
                ServeDir::new("./web-ui/src")
                    .precompressed_br()
                    .precompressed_gzip(),
            ),
    )
}

async fn set_static_cache_control(request: Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}
