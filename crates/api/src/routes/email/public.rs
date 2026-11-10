//! Public types for the email API
use serde::Deserialize;

// Thread shapes are produced by the library, so they live there and are
// re-exported here for API consumers.
pub use hq::types::email::{EmailMessage, EmailThread};

#[derive(Deserialize)]
pub struct EmailUnreadQuery {
    pub email: String,
    pub limit: Option<i64>,
}

#[derive(Deserialize)]
pub struct EmailSearchQuery {
    pub email: String,
    pub query: String,
}
