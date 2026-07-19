//! Approval API routes — lets the client respond to a pending
//! tool-call approval request issued by `ApprovalMiddleware`.

pub mod public;
mod router;

pub use router::router;