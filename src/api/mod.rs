pub mod routes;
mod server;
pub use server::{app, serve};
#[cfg(feature = "embed-assets")]
mod embed;
#[cfg(not(feature = "embed-assets"))]
mod disk;
mod static_assets;
pub mod public;
mod state;
pub use state::AppState;
mod utils;
