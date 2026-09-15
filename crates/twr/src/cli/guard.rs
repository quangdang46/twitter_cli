//! Guardrail regression tests (bead twitter_cli-5o3.1).
//!
//! twr is a safe read/write PRIMITIVE — never a summarizer, scheduler,
//! or approval workflow (plan §11). These tests pin that boundary so
//! scope creep fails the build instead of slipping in:
//! - no banned concept verbs in the command catalog,
//! - `future` stays a one-shot delay (cap + reads-only allowlist),
//! - no LLM/daemon/cron dependencies in the workspace.

/// Command names twr exposes (must mirror the `Command` enum + MCP catalog).
/// Adding a command requires updating this list AND justifying it here.
#[allow(dead_code)]
pub const COMMAND_NAMES: &[&str] = &[
    "status",
    "schema",
    "commands",
    "query-ids",
    "doctor",
    "login",
    "logout",
    "cache-search",
    "watch",
    "completions",
    "mcp",
    "post",
    "reply",
    "quote",
    "delete",
    "like",
    "unlike",
    "retweet",
    "unretweet",
    "bookmark",
    "unbookmark",
    "follow",
    "unfollow",
    "feed",
    "bookmarks",
    "search",
    "tweet",
    "show",
    "article",
    "list",
    "user",
    "user-posts",
    "likes",
    "followers",
    "following",
    "headlines",
    "future",
];

/// Verbs that must never become twr subcommands.
#[allow(dead_code)]
pub const BANNED_VERBS: &[&str] = &[
    "summarize",
    "summary",
    "digest",
    "rank",
    "schedule",
    "cron",
    "daemon",
    "approve",
    "approval",
    "orchestrate",
];

/// Banned dependency substrings (LLM SDKs, schedulers, embeddings).
#[allow(dead_code)]
pub const BANNED_DEPS: &[&str] = &[
    "openai",
    "anthropic",
    "google-generative",
    "mistral",
    "cohere",
    "ollama",
    "langchain",
    "llama-index",
    "tokio-cron",
    "cron_tab",
    "fastembed",
    "ort ",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_banned_command_verbs() {
        for cmd in COMMAND_NAMES {
            for banned in BANNED_VERBS {
                assert!(
                    !cmd.contains(banned),
                    "guardrail: command '{cmd}' contains banned verb '{banned}'"
                );
            }
        }
        // future is the one scheduling-adjacent primitive, and it is capped.
        assert!(COMMAND_NAMES.contains(&"future"));
        const { assert!(crate::MAX_FUTURE_DELAY_SECS <= 3600) }
        assert!(!crate::future_allowed("post"));
    }

    #[test]
    fn workspace_has_no_banned_dependencies() {
        let manifest = include_str!("../../../../Cargo.toml");
        let lock = include_str!("../../../../Cargo.lock");
        for dep in BANNED_DEPS {
            assert!(
                !manifest.to_lowercase().contains(dep),
                "guardrail: workspace manifest references banned dep '{dep}'"
            );
            assert!(
                !lock.to_lowercase().contains(&format!("name = \"{dep}\"")),
                "guardrail: Cargo.lock pulls banned dep '{dep}'"
            );
        }
    }
}
