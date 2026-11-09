pub mod ai;
pub mod anthropic;
pub mod channels;
pub mod core;
pub mod eval;
pub mod google;
pub mod jobs;
pub mod loops;
pub mod notify;
pub mod openai;
pub mod org;
pub mod search;
pub mod types;

// The vendored bashkit shell lives in its own crate (`hq-bash`). Re-export it
// here so the rest of the library keeps using `crate::bash::...`.
pub use hq_bash as bash;
