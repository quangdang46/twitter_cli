//! Session verification decision logic (plan §7).
//!
//! The live HTTP calls (`verify_credentials` → `settings.json` fallback
//! chain) belong to `twr-client`, not here — this module owns the *policy*
//! around them so it stays unit-testable without network:
//!
//! - [`classify_verify_status`]: map a verify-endpoint HTTP status to a
//!   [`VerifyOutcome`].
//! - [`should_reattempt_once`]: on 401/403 the caller re-runs browser
//!   extraction *exactly once* before failing with exit 77; every other
//!   outcome never re-extracts.

/// Outcome of one verify-endpoint probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// Session is alive.
    Valid,
    /// 401/403 — session dead or missing; exactly one re-extraction allowed.
    Unauthorized,
    /// 429 / inner 88/348/349 — back off, do NOT re-extract.
    RateLimited,
    /// Anything else (network error, 5xx, unexpected shape) — surfaced as-is.
    Inconclusive,
}

/// Map a verify-endpoint HTTP status to an outcome. `None` = the request
/// never completed (network error) → inconclusive.
pub fn classify_verify_status(status: Option<u16>) -> VerifyOutcome {
    match status {
        Some(200) => VerifyOutcome::Valid,
        Some(401) | Some(403) => VerifyOutcome::Unauthorized,
        Some(429) => VerifyOutcome::RateLimited,
        _ => VerifyOutcome::Inconclusive,
    }
}

/// True only when the caller should re-run browser extraction once and then
/// re-verify: exactly the 401/403 case, and only if no re-extraction has
/// happened yet for this invocation.
pub fn should_reattempt_once(outcome: VerifyOutcome, already_reattempted: bool) -> bool {
    outcome == VerifyOutcome::Unauthorized && !already_reattempted
}

/// Exit code for a terminal verify failure (plan §5.2): 77 = auth required.
pub const VERIFY_FAILURE_EXIT_CODE: i32 = 77;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_maps_to_outcome() {
        assert_eq!(classify_verify_status(Some(200)), VerifyOutcome::Valid);
        assert_eq!(
            classify_verify_status(Some(401)),
            VerifyOutcome::Unauthorized
        );
        assert_eq!(
            classify_verify_status(Some(403)),
            VerifyOutcome::Unauthorized
        );
        assert_eq!(
            classify_verify_status(Some(429)),
            VerifyOutcome::RateLimited
        );
        assert_eq!(
            classify_verify_status(Some(500)),
            VerifyOutcome::Inconclusive
        );
        assert_eq!(classify_verify_status(None), VerifyOutcome::Inconclusive);
    }

    #[test]
    fn exactly_one_reextraction_on_unauthorized() {
        assert!(should_reattempt_once(VerifyOutcome::Unauthorized, false));
        assert!(!should_reattempt_once(VerifyOutcome::Unauthorized, true));
        assert!(!should_reattempt_once(VerifyOutcome::Valid, false));
        assert!(!should_reattempt_once(VerifyOutcome::RateLimited, false));
        assert!(!should_reattempt_once(VerifyOutcome::Inconclusive, false));
    }
}
