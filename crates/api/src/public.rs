//! Public API types

use axum::response::{IntoResponse, Response};
use hq::google::oauth::AuthorizationError;
use http::StatusCode;

// Errors
pub struct ApiError(anyhow::Error);

/// Convert `AppError` into an Axum compatible response.
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        // A missing or expired OAuth grant is expected: the account just needs
        // to be connected. Warn rather than log a server error, and tell the
        // client it must re-authorize instead of reporting a 500.
        if let Some(auth) = self.0.downcast_ref::<AuthorizationError>() {
            tracing::warn!("{auth}");
            return (StatusCode::UNAUTHORIZED, auth.to_string()).into_response();
        }

        // Always log the error
        tracing::error!("{}\n{}", self.0, self.0.backtrace());

        // Respond with an error status
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

/// Enables using `?` on functions that return `Result<_,
/// anyhow::Error>` to turn them into `Result<_, AppError>`
impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

// Re-export public types from each route

pub mod calendar {
    pub use crate::routes::calendar::public::*;
}

pub mod channels {
    pub use crate::routes::channels::public::*;
}

pub mod email {
    pub use crate::routes::email::public::*;
}

pub mod files {
    pub use crate::routes::files::public::*;
}

pub mod loops {
    pub use crate::routes::loops::public::*;
}

pub mod metrics {
    pub use crate::routes::metrics::public::*;
}

pub mod notes {
    pub use crate::routes::notes::public::*;
}

pub mod push {
    pub use crate::routes::push::public::*;
}

pub mod webhook {
    pub use crate::routes::webhook::public::*;
}

pub mod skills {
    pub use crate::routes::skills::public::*;
}

pub mod web {
    pub use crate::routes::web::public::*;
}
