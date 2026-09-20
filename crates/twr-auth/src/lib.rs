//! Credential resolution for twr.
//!
//! Two independent paths, per plan §7 / SKILL.md:
//!   1. Browser cookie extraction via the `rookie` crate (Method A/B).
//!   2. A manual pasted cookie-string fallback (Method C), the mandatory
//!      safety net when browser extraction fails or is unavailable.
//!
//! SAFETY: nothing in this crate ever logs, prints, or transmits a real
//! cookie value. Extraction results are reported only as redacted booleans
//! (`found` / `*_present`). No network requests are made here.

pub mod guide;
pub mod resolve;
pub mod session;
pub mod verify;

pub use guide::{needs_first_run_wizard, FIRST_RUN_HINT, LOGIN_GUIDE};
pub use resolve::{
    browser_order, read_env, resolve, AuthSource, EnvInput, FlagInput, ResolvedAuth,
};
pub use session::{
    clear as clear_session, default_session_path, load as load_session, save as save_session,
    SaveError, SaveOutcome, SessionStatus,
};
pub use verify::{
    classify_verify_status, should_reattempt_once, VerifyOutcome, VERIFY_FAILURE_EXIT_CODE,
};

use std::collections::HashMap;

/// The two cookie names twr cares about for X/Twitter session auth.
pub const AUTH_TOKEN: &str = "auth_token";
pub const CT0: &str = "ct0";

/// Domains to look for cookies under. `x.com` is primary; `twitter.com` is
/// kept as a legacy alias since some sessions still carry cookies there.
pub const COOKIE_DOMAINS: &[&str] = &["x.com", "twitter.com"];

/// Which installed browser a cookie extraction attempt targeted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Browser {
    Chrome,
    Edge,
    Arc,
    Firefox,
    Brave,
}

impl Browser {
    pub fn name(&self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::Edge => "edge",
            Browser::Arc => "arc",
            Browser::Firefox => "firefox",
            Browser::Brave => "brave",
        }
    }

    /// Resolution order mirrors plan §7: Chrome first (most common), then
    /// Edge/Arc/Brave (Chromium-family), then Firefox last.
    pub fn resolution_order() -> [Browser; 5] {
        [
            Browser::Chrome,
            Browser::Edge,
            Browser::Arc,
            Browser::Brave,
            Browser::Firefox,
        ]
    }
}

/// Redacted outcome of a single browser's extraction attempt. Never holds a
/// cookie value, only presence booleans.
#[derive(Debug, Clone)]
pub struct BrowserAttempt {
    pub browser: Browser,
    pub error: Option<String>,
    pub auth_token_present: bool,
    pub ct0_present: bool,
}

impl BrowserAttempt {
    fn found(&self) -> bool {
        self.error.is_none() && self.auth_token_present && self.ct0_present
    }
}

/// Redacted summary of a full extraction sweep across browsers, suitable for
/// logging/printing/committing — contains no secret material.
#[derive(Debug, Clone)]
pub struct ExtractionSummary {
    pub attempts: Vec<BrowserAttempt>,
    pub found: bool,
    pub source: Option<&'static str>,
}

impl ExtractionSummary {
    pub fn from_attempts(attempts: Vec<BrowserAttempt>) -> Self {
        let hit = attempts.iter().find(|a| a.found());
        ExtractionSummary {
            found: hit.is_some(),
            source: hit.map(|a| a.browser.name()),
            attempts,
        }
    }
}

/// The (unredacted) extracted cookie values. Callers must never log/print
/// this struct's contents; only `ExtractionSummary` is safe to surface.
///
/// `full_string` carries the COMPLETE pasted cookie string when the session
/// came from a Method C paste (`--cookie` / `twr login --cookie`). It exists
/// because X's automated-behavior gate (code 226, live-confirmed 2026-09-16)
/// rejects write ops whose `Cookie:` header is only `auth_token;ct0` — a thin
/// context that looks automated. Forwarding the FULL browser cookie string
/// preserves the richer browser context and passes the same gate, exactly as
/// the Python original's "full cookie forwarding" design (which keeps every
/// cookie, not just the two it needs) documents. This is the single most
/// important field in this struct for write reliability — never drop it, and
/// never "normalize" a session down to just the pair.
#[derive(Debug, Clone, Default)]
pub struct SessionCookies {
    pub auth_token: Option<String>,
    pub ct0: Option<String>,
    /// Full original cookie string when available (Method C paste). Wins
    /// over the pair at header-build time (see
    /// `twr_client::Credentials::cookie_header`).
    pub full_string: Option<String>,
    /// Numeric account id decoded from the `twid=u=<id>` cookie when the
    /// session came from a Method C paste. Lets `twr lists` (and any future
    /// self-scoped command) resolve "self" without a REST round trip — the
    /// 1.1 `account/settings.json` / `verify_credentials` chain returns
    /// 401/403 with cookie auth even on live sessions, so it cannot be
    /// relied on. Never logged; informational only.
    pub twid_user_id: Option<String>,
}

impl SessionCookies {
    fn merge(&mut self, other: SessionCookies) {
        if self.auth_token.is_none() {
            self.auth_token = other.auth_token;
        }
        if self.ct0.is_none() {
            self.ct0 = other.ct0;
        }
        // A merged richer context is strictly better than a bare pair: the
        // pair-only path (env/flags) can trigger X's 226 gate on writes.
        if self.full_string.is_none() {
            self.full_string = other.full_string;
        }
        if self.twid_user_id.is_none() {
            self.twid_user_id = other.twid_user_id;
        }
    }

    pub fn is_complete(&self) -> bool {
        self.auth_token.is_some() && self.ct0.is_some()
    }

    /// Best-effort self user id: the `twid`-decoded id when available.
    pub fn self_user_id(&self) -> Option<&str> {
        self.twid_user_id.as_deref()
    }
}

/// Parse a full pasted cookie string (e.g. copied from browser devtools'
/// "Cookie" request header, or a `document.cookie` dump) into a name->value
/// map. This is Method C, the mandatory fallback regardless of whether
/// browser extraction (rookie) succeeds.
///
/// Format expected: `name1=value1; name2=value2; ...` (values are not
/// URL-decoded; whitespace around `;` and `=` is trimmed).
pub fn parse_cookie_string(s: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in s.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((name, value)) = part.split_once('=') {
            map.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    map
}

/// Build `SessionCookies` from a parsed cookie map (Method C entry point).
pub fn session_from_cookie_string(s: &str) -> SessionCookies {
    let map = parse_cookie_string(s);
    let twid_user_id = decode_twid_user_id(map.get("twid").map(String::as_str).unwrap_or(""));
    let session = SessionCookies {
        auth_token: map.get(AUTH_TOKEN).cloned(),
        ct0: map.get(CT0).cloned(),
        full_string: None,
        twid_user_id,
    };
    if !session.is_complete() {
        return session;
    }
    // Keep the whole paste (not just the pair) so writes forward a rich
    // browser cookie context — see the `full_string` field docs for why
    // this is load-bearing against X's code-226 automated-behavior gate.
    SessionCookies {
        full_string: Some(s.trim().to_string()),
        ..session
    }
}

/// Decode the numeric account id from a `twid` cookie value (`u=<id>`, the
/// value arriving percent-encoded as `u%3D<id>` in a raw paste — tolerate
/// both). Returns `None` for anything that is not all digits.
pub fn decode_twid_user_id(raw: &str) -> Option<String> {
    let v = raw.trim().trim_matches('"').trim();
    let v = v.replace("%3D", "=").replace("%3d", "=");
    let id = v.strip_prefix("u=").unwrap_or(&v).trim();
    if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
        Some(id.to_string())
    } else {
        None
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod rookie_backend {
    use super::*;

    fn cookies_for(
        browser: Browser,
        domains: Option<Vec<String>>,
    ) -> Result<Vec<rookie::common::enums::Cookie>, String> {
        let result = match browser {
            Browser::Chrome => rookie::chrome(domains),
            Browser::Edge => rookie::edge(domains),
            Browser::Arc => rookie::arc(domains),
            Browser::Firefox => rookie::firefox(domains),
            Browser::Brave => rookie::brave(domains),
        };
        result.map_err(|e| e.to_string())
    }

    /// Try one browser, returning a redacted `BrowserAttempt` plus the
    /// (unredacted, in-memory only) `SessionCookies` if found.
    pub fn try_browser(browser: Browser) -> (BrowserAttempt, SessionCookies) {
        let domains = Some(COOKIE_DOMAINS.iter().map(|d| d.to_string()).collect());
        match cookies_for(browser, domains) {
            Ok(cookies) => {
                let mut session = SessionCookies::default();
                for c in cookies {
                    match c.name.as_str() {
                        AUTH_TOKEN => session.auth_token = Some(c.value),
                        CT0 => session.ct0 = Some(c.value),
                        _ => {}
                    }
                }
                let attempt = BrowserAttempt {
                    browser,
                    error: None,
                    auth_token_present: session.auth_token.is_some(),
                    ct0_present: session.ct0.is_some(),
                };
                (attempt, session)
            }
            Err(e) => (
                BrowserAttempt {
                    browser,
                    error: Some(e.to_string()),
                    auth_token_present: false,
                    ct0_present: false,
                },
                SessionCookies::default(),
            ),
        }
    }
}

/// Sweep all supported browsers (in plan §7's resolution order), stopping
/// early once a complete `auth_token` + `ct0` pair is found. Returns both
/// the (unredacted, in-memory) cookies and a redacted summary safe to log.
///
/// On platforms `rookie` doesn't support, every browser attempt reports the
/// "unsupported platform" error and extraction always falls through to
/// Method C (`parse_cookie_string`).
pub fn extract_session_cookies() -> (SessionCookies, ExtractionSummary) {
    let mut session = SessionCookies::default();
    let mut attempts = Vec::new();

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        for browser in Browser::resolution_order() {
            let (attempt, found) = rookie_backend::try_browser(browser);
            session.merge(found);
            attempts.push(attempt);
            if session.is_complete() {
                break;
            }
        }
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        for browser in Browser::resolution_order() {
            attempts.push(BrowserAttempt {
                browser,
                error: Some("rookie: unsupported platform".to_string()),
                auth_token_present: false,
                ct0_present: false,
            });
        }
    }

    let summary = ExtractionSummary::from_attempts(attempts);
    (session, summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cookie_string_extracts_synthetic_pair() {
        // SYNTHETIC values only — never a real cookie.
        let raw = "auth_token=deadbeef00112233; ct0=abcdef0123456789; guest_id=v1%3A123";
        let map = parse_cookie_string(raw);
        assert_eq!(
            map.get(AUTH_TOKEN).map(String::as_str),
            Some("deadbeef00112233")
        );
        assert_eq!(map.get(CT0).map(String::as_str), Some("abcdef0123456789"));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn parse_cookie_string_handles_whitespace_and_empty_segments() {
        let raw = " auth_token = abc123 ;; ct0=xyz789 ; ";
        let map = parse_cookie_string(raw);
        assert_eq!(map.get(AUTH_TOKEN).map(String::as_str), Some("abc123"));
        assert_eq!(map.get(CT0).map(String::as_str), Some("xyz789"));
    }

    #[test]
    fn session_from_cookie_string_is_complete_with_synthetic_pair() {
        let raw = "auth_token=fake_synthetic_token; ct0=fake_synthetic_csrf";
        let session = session_from_cookie_string(raw);
        assert!(session.auth_token.is_some());
        assert!(session.ct0.is_some());
        assert!(session.is_complete());
    }

    #[test]
    fn twid_user_id_decodes_from_cookie_paste() {
        // SYNTHETIC values only — never a real cookie.
        let raw = "auth_token=fake_synthetic_token; ct0=fake_synthetic_csrf; twid=u%3D1758783014887124992";
        let session = session_from_cookie_string(raw);
        assert_eq!(
            session.twid_user_id.as_deref(),
            Some("1758783014887124992")
        );
        assert_eq!(session.self_user_id(), Some("1758783014887124992"));
        // Unencoded form + junk both tolerated.
        assert_eq!(decode_twid_user_id("u=12345"), Some("12345".into()));
        assert_eq!(decode_twid_user_id("not-an-id"), None);
        assert_eq!(decode_twid_user_id(""), None);
    }

    #[test]
    fn session_from_cookie_string_incomplete_when_missing_field() {
        let raw = "auth_token=only_this_one";
        let session = session_from_cookie_string(raw);
        assert!(session.auth_token.is_some());
        assert!(session.ct0.is_none());
        assert!(!session.is_complete());
    }

    /// This is the actual spike acceptance check: run real extraction on
    /// THIS machine and assert only redacted facts. Never asserts on or
    /// prints cookie values.
    #[test]
    fn live_extraction_spike_on_this_machine() {
        let (_session, summary) = extract_session_cookies();
        // Print only the redacted summary (safe for CI/test logs).
        eprintln!(
            "twr-auth spike: found={} source={:?} attempts={}",
            summary.found,
            summary.source,
            summary.attempts.len()
        );
        for a in &summary.attempts {
            eprintln!(
                "  browser={} auth_token_present={} ct0_present={} error={:?}",
                a.browser.name(),
                a.auth_token_present,
                a.ct0_present,
                a.error
            );
        }
        // We assert the sweep runs to completion and reports one attempt
        // per supported browser; we do NOT assert `found` since whether
        // this machine is logged into x.com in a supported browser is an
        // environment fact, not a crate-correctness fact.
        assert_eq!(summary.attempts.len(), Browser::resolution_order().len());
    }
}
