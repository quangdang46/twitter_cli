//! `twr-core` — shared types for the twr workspace.
//!
//! This crate currently holds only the envelope and error skeleton described
//! in `PLAN.md` §5 (Agent Contract). Implementation lands in Phase 1 (P1).

pub mod envelope;
pub mod error;

pub use envelope::Envelope;
pub use error::{ErrorKind, TwrError};
