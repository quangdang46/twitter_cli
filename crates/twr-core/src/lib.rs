//! `twr-core` — shared types for the twr workspace.
//!
//! Envelope + error + output-flag contract from `PLAN.md` §5 (Agent Contract).

pub mod apply;
pub mod budget;
pub mod envelope;
pub mod error;
pub mod idempotency;
pub mod output;
pub mod policy;
pub mod timefmt;

pub use apply::{ambiguous_message, cancelled_data, decide, dry_run_data, ApplyInput, Decision};
pub use budget::{
    BudgetCheck, MutationLog, BUDGET_ENV_VAR, DEFAULT_DAILY_BUDGET, MIN_DAILY_BUDGET,
};
pub use envelope::{
    apply_compact, apply_fields, emit, parse_fields, render_json, Envelope, Meta, Pagination,
};
pub use error::{
    classify_api_code, is_not_found_payload, is_secret_flag, ErrorKind, FailingInput, TwrError,
    RATE_LIMIT_API_CODES, REDACTED, SECRET_FLAGS,
};
pub use idempotency::{
    IdempotencyEntry, IdempotencyStore, PreCheck, WriteState, IDEMPOTENCY_TTL_SECS,
    UNKNOWN_SUGGESTION,
};
pub use output::{new_trace_id, OutputFormat, OutputOptions};
pub use policy::Policy;
pub use timefmt::{absolute, display, parse_twitter_time, relative, TimeMode};
