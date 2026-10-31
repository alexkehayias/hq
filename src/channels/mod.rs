//! Channels: named streams of events that loops and other agents subscribe to.
//!
//! A channel has a [`ChannelSource`] describing how it is served — a Unix socket
//! publisher (`Socket`) or an in-process server publisher (`Server`, e.g. the
//! default timer channels and webhooks) — and an optional owner (`created_by`),
//! the agent session that created it via tools. The two are independent: an
//! agent-created channel is a `Server`-sourced channel that happens to have an
//! owner. See [`models`] for the types, [`db`] for persistence, [`registry`]
//! for the in-process bus, [`subscribe`] for source-aware subscription,
//! [`default`] for the built-in timer channels, [`errors`] for the default
//! channel carrying the server's error logs, and [`notes`] for the default
//! channel announcing newly indexed notes.

pub mod db;
pub mod default;
pub mod errors;
pub mod models;
pub mod notes;
pub mod prompt;
pub mod registry;
pub mod subscribe;

pub use models::{Channel, ChannelSource};
