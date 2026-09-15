//! Error kinds and the exit-code contract. See `PLAN.md` §5.2.
//!
//! This table is frozen after Phase 1 (P1) — do not renumber once shipped.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    GeneralAuth,
    UsagePolicyDenied,
    NotFound,
    ForbiddenRateLimited,
    Network,
    ContractDrift,
    AttachmentIo,
    AuthRequired,
}

impl ErrorKind {
    /// Exit code contract — see PLAN.md §5.2.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::GeneralAuth => 1,
            ErrorKind::UsagePolicyDenied => 2,
            ErrorKind::NotFound => 3,
            ErrorKind::ForbiddenRateLimited => 4,
            ErrorKind::Network => 5,
            ErrorKind::ContractDrift => 6,
            ErrorKind::AttachmentIo => 7,
            ErrorKind::AuthRequired => 77,
        }
    }

    pub fn is_retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::ForbiddenRateLimited | ErrorKind::Network | ErrorKind::ContractDrift
        )
    }
}

#[derive(Debug, Error, Serialize)]
#[error("{message}")]
pub struct TwrError {
    pub code: ErrorKind,
    pub message: String,
    pub suggestion: Option<String>,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
    pub failing_input: Option<String>,
}

impl TwrError {
    pub fn new(code: ErrorKind, message: impl Into<String>) -> Self {
        let retryable = code.is_retryable();
        Self {
            code,
            message: message.into(),
            suggestion: None,
            retryable,
            retry_after_ms: None,
            failing_input: None,
        }
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}
