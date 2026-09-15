//! `twr-core` — shared types for the twr workspace.
//!
//! Envelope + error + output-flag contract from `PLAN.md` §5 (Agent Contract).

pub mod apply;
pub mod envelope;
pub mod error;
pub mod output;

pub use apply::{ambiguous_message, cancelled_data, decide, dry_run_data, ApplyInput, Decision};
pub use envelope::{
    apply_compact, apply_fields, emit, parse_fields, render_json, Envelope, Meta, Pagination,
};
pub use error::{
    classify_api_code, is_not_found_payload, is_secret_flag, ErrorKind, FailingInput, TwrError,
    RATE_LIMIT_API_CODES, REDACTED, SECRET_FLAGS,
};
pub use output::{new_trace_id, OutputFormat, OutputOptions};
