pub mod routes;
mod server;
pub use server::{app, serve};
#[cfg(feature = "embed-assets")]
mod embed;
pub mod public;
mod state;
pub use state::AppState;
mod utils;
