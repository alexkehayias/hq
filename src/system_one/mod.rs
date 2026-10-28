//! Client and types for the System One HTTP API.
//!
//! System One models answer typed questions about a `state` and return
//! structured answers that code can branch on directly. The [`SystemOneClient`]
//! talks to any server exposing the `/v1/systemone` and `/v1/models` endpoints,
//! including a locally hosted inference server.

pub mod client;
pub mod models;

pub use client::*;
pub use models::*;
