//! Webhook API routes

pub mod public;
mod router;

pub use router::{github_router, router};
