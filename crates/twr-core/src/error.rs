//! Error kinds and the exit-code contract. See `PLAN.md` §5.2 and §7.
//!
//! This table is frozen after Phase 1 (P1) — do not renumber once shipped.
//!
//! Classification rule (§5.2 disambiguation): the two different "404"s are
//! distinguished by *payload shape*, never by HTTP status alone — an HTTP 404
//! on the GraphQL endpoint itself means the query ID is stale (contract
//! drift, exit 6), while an HTTP 200 whose payload is a
//! tombstone/`*Unavailable` result means the *target* is missing (not-found,
//! exit 3). See [`ErrorKind::from_http_status`] + [`is_not_found_payload`].

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// Flags whose values are secrets. Their values MUST never survive into
/// [`TwrError::failing_input`], logs, traces, or envelopes (plan §0.1 #8,
/// §5.3). Construction enforces this — see [`FailingInput::new`].
pub const SECRET_FLAGS: &[&str] = &[
    "--cookie",
    "--auth-token",
    "--ct0",
    "--proxy",
    "--proxy-with-credentials",
];

/// Redaction sentinel written in place of any secret value.
pub const REDACTED: &str = "[REDACTED]";

/// Returns true if `flag` carries a secret value (exact match against
/// [`SECRET_FLAGS`).
pub fn is_secret_flag(flag: &str) -> bool {
    SECRET_FLAGS.contains(&flag)
}

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

    /// Default agent-facing recovery suggestion per plan §5.2's exit table.
    pub fn default_suggestion(self) -> &'static str {
        match self {
            ErrorKind::GeneralAuth => "run `twr status`, fix per suggestion; do not blind-retry",
            ErrorKind::UsagePolicyDenied => {
                "fix args or change policy; writes need --apply (see `twr status`)"
            }
            ErrorKind::NotFound => "verify the ID/handle, do not retry",
            ErrorKind::ForbiddenRateLimited => {
                "back off retry_after_ms then resume with --cursor; partial data ships with meta.truncated=true"
            }
            ErrorKind::Network => "retry with exponential backoff",
            ErrorKind::ContractDrift => "run `twr doctor --refresh` once, then retry once",
            ErrorKind::AttachmentIo => "fix the path/permissions/size",
            ErrorKind::AuthRequired => "run `twr status`, then log in; do not blind-retry",
        }
    }

    /// Map an HTTP transport status to a kind. Payload-shape cases (tombstone
    /// vs. stale query ID) are NOT decided here — see [`is_not_found_payload`]
    /// and the note on [`from_http_status`] about 404.
    pub fn from_http_status(status: u16) -> Self {
        match status {
            401 | 403 => ErrorKind::AuthRequired,
            404 => ErrorKind::ContractDrift,
            226 => ErrorKind::ForbiddenRateLimited,
            429 => ErrorKind::ForbiddenRateLimited,
            _ if (500..600).contains(&status) => ErrorKind::Network,
            _ => ErrorKind::GeneralAuth,
        }
    }

    /// Map an X API inner error code (the `code` field inside a 200/4xx error
    /// payload) to a kind. Known rate-limit codes: 88, 348, 349.
    pub fn from_api_code(code: i64) -> Self {
        match code {
            88 | 348 | 349 => ErrorKind::ForbiddenRateLimited,
            _ => ErrorKind::GeneralAuth,
        }
    }
}

/// Inner X API error codes that mean rate-limited (exit 4), per plan §1.2.
pub const RATE_LIMIT_API_CODES: &[i64] = &[88, 348, 349];

/// Classify an error payload's inner `code` field. Returns
/// `ForbiddenRateLimited` for 88/348/349, `GeneralAuth` otherwise.
pub fn classify_api_code(code: i64) -> ErrorKind {
    ErrorKind::from_api_code(code)
}

/// Payload-shape classifier for the §5.2 "two 404s" disambiguation: returns
/// true when an HTTP-200 response body says the *target* doesn't exist (a
/// `TweetTombstone` / `TweetUnavailable` / `UserUnavailable` result, or a
/// `tombstone` object) — i.e. exit 3 not-found, NOT exit 6 contract drift.
pub fn is_not_found_payload(payload: &Value) -> bool {
    const TOMBSTONE_TYPES: &[&str] = &["TweetTombstone", "TweetUnavailable", "UserUnavailable"];
    fn walk(v: &Value) -> bool {
        match v {
            Value::Object(map) => {
                if map
                    .get("__typename")
                    .and_then(Value::as_str)
                    .is_some_and(|t| TOMBSTONE_TYPES.contains(&t))
                {
                    return true;
                }
                if map.contains_key("tombstone") {
                    return true;
                }
                map.values().any(walk)
            }
            Value::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    walk(payload)
}

/// A pre-redacted failing input. Invariant (plan §0.1 #8, §5.3): secret-flag
/// values are replaced with [`REDACTED`] at construction time, inside this
/// type — never at call sites, never from raw argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FailingInput {
    pub flag: String,
    pub value: String,
}

impl FailingInput {
    /// Build a failing input, forcing secret values to `[REDACTED]`.
    pub fn new(flag: impl Into<String>, value: impl Into<String>) -> Self {
        let flag = flag.into();
        let value = if is_secret_flag(&flag) {
            REDACTED.to_string()
        } else {
            value.into()
        };
        Self { flag, value }
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
    pub failing_input: Option<FailingInput>,
}

impl TwrError {
    pub fn new(code: ErrorKind, message: impl Into<String>) -> Self {
        let retryable = code.is_retryable();
        let suggestion = Some(code.default_suggestion().to_string());
        Self {
            code,
            message: message.into(),
            suggestion,
            retryable,
            retry_after_ms: None,
            failing_input: None,
        }
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn with_retry_after_ms(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
    }

    /// Attach a failing input. Secret-flag values are redacted inside
    /// [`FailingInput::new`]; callers pass the raw value and CANNOT leak it.
    pub fn with_failing_input(mut self, flag: impl Into<String>, value: impl Into<String>) -> Self {
        self.failing_input = Some(FailingInput::new(flag, value));
        self
    }

    /// Build an auth-required (401/403) error.
    pub fn auth_required(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::AuthRequired, message)
    }

    /// Build a rate-limit (429 / inner 88/348/349) error.
    pub fn rate_limited(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ForbiddenRateLimited, message)
    }

    /// Build a stale-query-ID (HTTP 404 on the endpoint) error.
    pub fn contract_drift(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ContractDrift, message)
    }

    /// Build a missing-target (tombstone / `*Unavailable` payload) error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    /// Build an automated-behavior (HTTP 226) error. Exit 4 like other
    /// rate limits; per bird's pattern the suggestion points at the legacy
    /// `statuses/update.json` fallback for writes.
    pub fn automated_behavior(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ForbiddenRateLimited, message).with_suggestion(
            "automated-behavior detected (HTTP 226); back off, and for writes fall back to the legacy statuses/update.json endpoint",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_401_maps_to_auth_required() {
        assert_eq!(ErrorKind::from_http_status(401), ErrorKind::AuthRequired);
        assert_eq!(ErrorKind::AuthRequired.exit_code(), 77);
    }

    #[test]
    fn status_403_maps_to_auth_required() {
        assert_eq!(ErrorKind::from_http_status(403), ErrorKind::AuthRequired);
    }

    #[test]
    fn status_429_maps_to_rate_limited() {
        assert_eq!(
            ErrorKind::from_http_status(429),
            ErrorKind::ForbiddenRateLimited
        );
        assert_eq!(ErrorKind::ForbiddenRateLimited.exit_code(), 4);
    }

    #[test]
    fn inner_codes_88_348_349_map_to_rate_limited() {
        for code in [88, 348, 349] {
            assert_eq!(classify_api_code(code), ErrorKind::ForbiddenRateLimited);
            assert_eq!(
                ErrorKind::from_api_code(code),
                ErrorKind::ForbiddenRateLimited
            );
        }
    }

    #[test]
    fn endpoint_404_maps_to_contract_drift_not_not_found() {
        // The §5.2 disambiguation: transport 404 = stale query ID = exit 6.
        assert_eq!(ErrorKind::from_http_status(404), ErrorKind::ContractDrift);
        assert_eq!(ErrorKind::ContractDrift.exit_code(), 6);
    }

    #[test]
    fn tombstone_payload_classifies_as_not_found() {
        let payload = json!({
            "data": { "tweetResult": { "result": { "__typename": "TweetTombstone" } } }
        });
        assert!(is_not_found_payload(&payload));
        assert_eq!(ErrorKind::NotFound.exit_code(), 3);
    }

    #[test]
    fn unavailable_payloads_classify_as_not_found() {
        for typename in ["TweetUnavailable", "UserUnavailable"] {
            let payload = json!({ "result": { "__typename": typename } });
            assert!(is_not_found_payload(&payload), "{typename}");
        }
    }

    #[test]
    fn healthy_payload_does_not_classify_as_not_found() {
        let payload = json!({ "result": { "__typename": "Tweet", "rest_id": "123" } });
        assert!(!is_not_found_payload(&payload));
    }

    #[test]
    fn status_226_maps_to_rate_limited_with_legacy_fallback_suggestion() {
        assert_eq!(
            ErrorKind::from_http_status(226),
            ErrorKind::ForbiddenRateLimited
        );
        let err = TwrError::automated_behavior("automated");
        assert_eq!(err.code, ErrorKind::ForbiddenRateLimited);
        let suggestion = err.suggestion.unwrap();
        assert!(suggestion.contains("statuses/update.json"), "{suggestion}");
    }

    #[test]
    fn secret_flag_values_never_survive_serialization() {
        for flag in SECRET_FLAGS.iter().copied() {
            let err = TwrError::new(ErrorKind::AuthRequired, "nope")
                .with_failing_input(flag, "super-secret-live-value");
            let serialized = serde_json::to_string(&err).unwrap();
            assert!(
                !serialized.contains("super-secret-live-value"),
                "leak via {flag}: {serialized}"
            );
            assert!(serialized.contains(REDACTED), "{serialized}");
        }
    }

    #[test]
    fn non_secret_flag_values_survive() {
        let err = TwrError::new(ErrorKind::NotFound, "missing").with_failing_input("--max", "500");
        let input = err.failing_input.unwrap();
        assert_eq!(input.flag, "--max");
        assert_eq!(input.value, "500");
    }

    #[test]
    fn every_kind_has_exit_code_and_suggestion() {
        let kinds = [
            ErrorKind::GeneralAuth,
            ErrorKind::UsagePolicyDenied,
            ErrorKind::NotFound,
            ErrorKind::ForbiddenRateLimited,
            ErrorKind::Network,
            ErrorKind::ContractDrift,
            ErrorKind::AttachmentIo,
            ErrorKind::AuthRequired,
        ];
        let codes: Vec<i32> = kinds.iter().map(|k| k.exit_code()).collect();
        assert_eq!(codes, vec![1, 2, 3, 4, 5, 6, 7, 77]);
        for kind in kinds {
            assert!(!kind.default_suggestion().is_empty());
            let err = TwrError::new(kind, "m");
            assert!(err.suggestion.is_some());
            assert_eq!(err.retryable, kind.is_retryable());
        }
    }
}
