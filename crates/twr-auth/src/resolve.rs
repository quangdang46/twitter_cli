//! Credential resolution chain (plan §7).
//!
//! Order: CLI flags → env (`TWITTER_AUTH_TOKEN` + `TWITTER_CT0`) →
//! file (`~/.twr/session.json`) → browser extraction via rookie.
//!
//! Browser selection honors `TWITTER_BROWSER` (a single browser name, or
//! `"auto"`/unset for the default order) and `TWITTER_CHROME_PROFILE` is
//! documented where Chrome profiles matter (see [`browser_order`]).
//! The manual cookie-string paste (Method C) is always available regardless
//! of rookie's platform support — see [`crate::session_from_cookie_string`].

use crate::{session_from_cookie_string, Browser, SessionCookies};

/// Where a resolved credential pair came from. Not a secret — safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    Flags,
    Env,
    File,
    Browser(&'static str),
    CookieString,
}

impl AuthSource {
    pub fn name(&self) -> &'static str {
        match self {
            AuthSource::Flags => "flags",
            AuthSource::Env => "env",
            AuthSource::File => "file",
            AuthSource::Browser(b) => b,
            AuthSource::CookieString => "cookie-string",
        }
    }
}

/// A resolved credential pair plus its (non-secret) provenance.
#[derive(Debug, Clone)]
pub struct ResolvedAuth {
    /// Redacted provenance — safe to log/print.
    pub source: AuthSource,
    /// The live values. Callers must never log/print these.
    pub session: SessionCookies,
}

/// Explicit flag inputs (e.g. `--auth-token/--ct0` or `--cookie`).
#[derive(Debug, Clone, Default)]
pub struct FlagInput {
    pub auth_token: Option<String>,
    pub ct0: Option<String>,
    /// Full pasted cookie string via `--cookie`.
    pub cookie: Option<String>,
}

impl FlagInput {
    pub fn is_empty(&self) -> bool {
        self.auth_token.is_none() && self.ct0.is_none() && self.cookie.is_none()
    }
}

/// Explicit env snapshot, so resolution is testable without touching the real
/// process environment. Production callers build it via [`read_env`].
#[derive(Debug, Clone, Default)]
pub struct EnvInput {
    pub auth_token: Option<String>,
    pub ct0: Option<String>,
    /// `TWITTER_BROWSER`: single browser name or `"auto"`/unset.
    pub browser: Option<String>,
    /// `TWITTER_CHROME_PROFILE`: multi-profile Chrome selector (informational;
    /// rookie reads the default profile — this value is surfaced in
    /// diagnostics so agents know which profile was *requested*).
    pub chrome_profile: Option<String>,
}

/// Read the `TWITTER_*` variables from the process environment.
pub fn read_env() -> EnvInput {
    EnvInput {
        auth_token: std::env::var("TWITTER_AUTH_TOKEN")
            .ok()
            .filter(|v| !v.is_empty()),
        ct0: std::env::var("TWITTER_CT0").ok().filter(|v| !v.is_empty()),
        browser: std::env::var("TWITTER_BROWSER")
            .ok()
            .filter(|v| !v.is_empty()),
        chrome_profile: std::env::var("TWITTER_CHROME_PROFILE")
            .ok()
            .filter(|v| !v.is_empty()),
    }
}

/// Parse a `TWITTER_BROWSER` value into an ordered browser list.
/// `"auto"`/empty/unknown → default [`Browser::resolution_order`]; a single
/// known name → that browser first, then the rest in default order (so a
/// miss still falls through instead of hard-failing).
pub fn browser_order(selection: Option<&str>) -> Vec<Browser> {
    let default = Browser::resolution_order();
    let want = selection
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "auto");
    let Some(want) = want else {
        return default.to_vec();
    };
    let first = default.iter().find(|b| b.name().eq_ignore_ascii_case(want));
    match first {
        Some(first) => {
            let mut order = vec![*first];
            order.extend(default.iter().filter(|b| b != &first).copied());
            order
        }
        None => default.to_vec(),
    }
}

/// Resolve credentials through the chain. `extract` is the browser sweep
/// (usually [`crate::extract_session_cookies`}); injecting it keeps this
/// pure-logic function unit-testable without touching real browser stores.
///
/// Precedence: flags → env → file → browser. Returns `None` when nothing in
/// the chain yields a complete `auth_token` + `ct0` pair.
pub fn resolve(
    flags: &FlagInput,
    env: &EnvInput,
    file: Option<SessionCookies>,
    extract: impl FnOnce() -> (SessionCookies, Vec<Browser>),
) -> Option<ResolvedAuth> {
    // 1. CLI flags: explicit --auth-token/--ct0, or a --cookie paste.
    if !flags.is_empty() {
        let session = if flags.cookie.is_some() {
            session_from_cookie_string(flags.cookie.as_deref().unwrap_or(""))
        } else {
            SessionCookies {
                auth_token: flags.auth_token.clone(),
                ct0: flags.ct0.clone(),
                full_string: None,
                twid_user_id: None,
            }
        };
        if session.is_complete() {
            return Some(ResolvedAuth {
                source: AuthSource::Flags,
                session,
            });
        }
        // Partial flags must NOT silently merge with lower layers — a half
        // flag pair is a usage error the caller should report, not paper over.
        return None;
    }

    // 2. Env pair.
    if let (Some(auth_token), Some(ct0)) = (env.auth_token.clone(), env.ct0.clone()) {
        return Some(ResolvedAuth {
            source: AuthSource::Env,
            session: SessionCookies {
                auth_token: Some(auth_token),
                ct0: Some(ct0),
                full_string: None,
                twid_user_id: None,
            },
        });
    }

    // 3. Session file.
    if let Some(session) = file {
        if session.is_complete() {
            return Some(ResolvedAuth {
                source: AuthSource::File,
                session,
            });
        }
    }

    // 4. Browser extraction.
    let (session, _attempted) = extract();
    if session.is_complete() {
        // Provenance detail (which browser hit) comes from the sweep summary;
        // here we record the generic browser source.
        return Some(ResolvedAuth {
            source: AuthSource::Browser("browser"),
            session,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(auth: &str, ct0: &str) -> SessionCookies {
        SessionCookies {
            auth_token: Some(auth.to_string()),
            ct0: Some(ct0.to_string()),
            full_string: None,
            twid_user_id: None,
        }
    }

    fn no_extract() -> (SessionCookies, Vec<Browser>) {
        (SessionCookies::default(), vec![])
    }

    #[test]
    fn flags_beat_env_file_and_browser() {
        let flags = FlagInput {
            auth_token: Some("flag-token".into()),
            ct0: Some("flag-ct0".into()),
            cookie: None,
        };
        let env = EnvInput {
            auth_token: Some("env-token".into()),
            ct0: Some("env-ct0".into()),
            ..Default::default()
        };
        let out = resolve(&flags, &env, Some(complete("file", "file")), no_extract).unwrap();
        assert_eq!(out.source, AuthSource::Flags);
        assert_eq!(out.session.auth_token.unwrap(), "flag-token");
    }

    #[test]
    fn cookie_flag_parses_method_c() {
        let flags = FlagInput {
            cookie: Some("auth_token=abc; ct0=xyz".into()),
            ..Default::default()
        };
        let out = resolve(&flags, &EnvInput::default(), None, no_extract).unwrap();
        assert_eq!(out.source, AuthSource::Flags);
        assert!(out.session.is_complete());
    }

    #[test]
    fn partial_flags_do_not_fall_through() {
        // A lone --auth-token must fail loudly, not silently merge with env.
        let flags = FlagInput {
            auth_token: Some("only-this".into()),
            ..Default::default()
        };
        let env = EnvInput {
            auth_token: Some("env-token".into()),
            ct0: Some("env-ct0".into()),
            ..Default::default()
        };
        assert!(resolve(&flags, &env, None, no_extract).is_none());
    }

    #[test]
    fn env_beats_file_and_browser() {
        let env = EnvInput {
            auth_token: Some("env-token".into()),
            ct0: Some("env-ct0".into()),
            ..Default::default()
        };
        let out = resolve(
            &FlagInput::default(),
            &env,
            Some(complete("file", "file")),
            || (complete("b", "b"), vec![]),
        )
        .unwrap();
        assert_eq!(out.source, AuthSource::Env);
    }

    #[test]
    fn file_beats_browser() {
        let out = resolve(
            &FlagInput::default(),
            &EnvInput::default(),
            Some(complete("file-t", "file-c")),
            || (complete("b-t", "b-c"), vec![]),
        )
        .unwrap();
        assert_eq!(out.source, AuthSource::File);
        assert_eq!(out.session.auth_token.unwrap(), "file-t");
    }

    #[test]
    fn browser_is_last_resort_and_none_when_all_empty() {
        let out = resolve(&FlagInput::default(), &EnvInput::default(), None, || {
            (complete("b-t", "b-c"), vec![])
        })
        .unwrap();
        assert!(matches!(out.source, AuthSource::Browser(_)));
        assert!(resolve(
            &FlagInput::default(),
            &EnvInput::default(),
            None,
            no_extract
        )
        .is_none());
    }

    #[test]
    fn browser_order_defaults_and_prefers_selection() {
        assert_eq!(browser_order(None).len(), 5);
        assert_eq!(browser_order(Some("auto")).len(), 5);
        assert_eq!(browser_order(Some("nonsense"))[0], Browser::Chrome);
        let order = browser_order(Some("firefox"));
        assert_eq!(order[0], Browser::Firefox);
        assert_eq!(order.len(), 5);
        let order = browser_order(Some("EDGE"));
        assert_eq!(order[0], Browser::Edge);
    }
}
