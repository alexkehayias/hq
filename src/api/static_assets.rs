//! Static asset serving.
//!
//! Prod builds (`--features embed-assets`) serve the web UI embedded in the
//! binary. Dev builds serve `web-ui/src` from disk so changes appear on reload.

#[cfg(feature = "embed-assets")]
pub use crate::api::embed::attach_assets;

#[cfg(not(feature = "embed-assets"))]
pub use crate::api::disk::attach_assets;
