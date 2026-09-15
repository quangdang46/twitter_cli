//! `twr login --guide` text + first-run wizard trigger (upstream issue #46).
//!
//! Issue #46: users couldn't tell *where* to set credentials. The guide
//! prints all three methods with doc links; the wizard fires when
//! `twr status` finds no session at all. Both are pure text here — the CLI
//! crate decides when to print them.

/// Printed by `twr login --guide`. Method C (paste) is always listed last
/// because it works everywhere rookie doesn't.
pub const LOGIN_GUIDE: &str = "\
twr login — three ways to authenticate (pick one):

  Method A — environment variables (CI / containers):
    export TWITTER_AUTH_TOKEN='<auth_token cookie value>'
    export TWITTER_CT0='<ct0 cookie value>'

  Method B — browser extraction (local dev, Chrome/Edge/Arc/Brave/Firefox):
    Log into x.com in your browser, then run `twr login`.
    Optional: TWITTER_BROWSER=chrome|edge|arc|brave|firefox (default: auto),
    TWITTER_CHROME_PROFILE='<profile>' for multi-profile Chrome.

  Method C — paste a full cookie string (always works):
    Copy the Cookie request header from browser devtools on x.com,
    then run `twr login --cookie '<pastestring>'`.

Docs: https://github.com/quangdang46/twitter_cli#auth
";

/// Printed when `twr status` finds no session at all (first-run wizard).
pub const FIRST_RUN_HINT: &str = "\
No X/Twitter session found. Run `twr login --guide` to pick one of three \
auth methods (env, browser extraction, or cookie paste), then `twr status` \
again to verify.\
";

/// True when the wizard should fire: no source in the chain resolved.
pub fn needs_first_run_wizard(resolved_source: Option<&str>) -> bool {
    resolved_source.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_mentions_all_three_methods() {
        assert!(LOGIN_GUIDE.contains("TWITTER_AUTH_TOKEN"));
        assert!(LOGIN_GUIDE.contains("TWITTER_BROWSER"));
        assert!(LOGIN_GUIDE.contains("--cookie"));
    }

    #[test]
    fn wizard_fires_only_when_nothing_resolved() {
        assert!(needs_first_run_wizard(None));
        assert!(!needs_first_run_wizard(Some("env")));
    }
}
