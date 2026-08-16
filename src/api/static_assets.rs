//! Static asset serving.
//!
//! Prod builds (`--features embed-assets`) serve the web UI embedded in the
//! binary. Dev builds serve `web-ui/src` from disk so changes appear on reload.

use axum::Router;

#[cfg(feature = "embed-assets")]
use super::embed;

#[cfg(not(feature = "embed-assets"))]
use axum::middleware;
#[cfg(not(feature = "embed-assets"))]
use axum::extract::Request;
#[cfg(not(feature = "embed-assets"))]
use axum::response::Response;
#[cfg(not(feature = "embed-assets"))]
use http::{HeaderValue, header};
#[cfg(not(feature = "embed-assets"))]
use tower::ServiceBuilder;
#[cfg(not(feature = "embed-assets"))]
use tower_http::services::ServeDir;

/// Attach the static-asset fallback to `router`.
#[cfg(feature = "embed-assets")]
pub fn attach_assets<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.fallback(embed::handler)
}

/// Attach the static-asset fallback to `router`.
#[cfg(not(feature = "embed-assets"))]
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

#[cfg(not(feature = "embed-assets"))]
async fn set_static_cache_control(request: Request, next: middleware::Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}
