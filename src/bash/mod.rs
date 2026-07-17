//! Vendored bashkit — virtual bash shell with POSIX filesystem.
//!
//! Trimmed from bashkit v0.12 to the subset used by `hq`. See `LICENSE` in
//! this directory for upstream attribution (MIT, Mykhailo Chaliy / Everruns).
//!
//! Public API surface (what `hq` imports):
//! - [`Bash`] — virtual shell with `exec()`, `mount()`, custom builtins
//! - [`Builtin`] trait + [`Context`] (a.k.a. `BuiltinContext`) for custom commands
//! - [`ExecResult`] — stdout/stderr/exit_code/truncation
//! - [`PosixFs`], [`RealFs`] — filesystem backends for the sandbox
//!
//! This module is populated incrementally; see `.claude/plans/warm-twirling-stonebraker.md`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod builtins;
pub mod error;
pub mod fs;
pub mod hooks;
pub mod interpreter;
pub mod lib_core;
pub mod limits;
pub mod parser;
pub mod testing;
pub mod trace;

// Re-export the public API surface that `hq` imports (matches bashkit's).
pub use crate::bash::builtins::{Builtin, Context as BuiltinContext};
pub use crate::bash::error::{Error, Result};
pub use crate::bash::fs::{PosixFs, RealFs, RealFsMode};
pub use crate::bash::interpreter::ExecResult;
pub use crate::bash::lib_core::{Bash, BashBuilder, ExecOptions};

// Re-export async_trait so `use crate::bash::async_trait` works (bashkit did this).
pub use async_trait::async_trait;