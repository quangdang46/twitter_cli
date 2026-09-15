//! `twr-core` — shared types for the twr workspace.
//!
//! This crate currently holds only the envelope and error skeleton described
//! in `PLAN.md` §5 (Agent Contract). Implementation lands in Phase 1 (P1).

pub mod envelope;
pub mod error;

pub use envelope::Envelope;
pub use error::{
    classify_api_code, is_not_found_payload, is_secret_flag, ErrorKind, FailingInput, TwrError,
    RATE_LIMIT_API_CODES, REDACTED, SECRET_FLAGS,
};
