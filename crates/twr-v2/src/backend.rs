//! Dual-backend routing (bead twitter_cli-5o3.6.2).
//!
//! `--backend api-v2` alongside the default cookie backend. Per-command
//! routing (PR #31 scope): cookie-only for feed/bookmarks (the official API
//! exposes no equivalent); either backend where v2 has coverage
//! (search/post/user/tweet…); v2 adds `search --scope recent|all` and
//! `tweet/show --reply-scope auto|recent|all`.
//!
//! The P1 `HttpTransport` abstraction makes this additive: v2 calls go
//! through the same transport with a Bearer header instead of cookies.

/// Which backend serves a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// Cookie GraphQL (default).
    #[default]
    Cookie,
    /// Official API v2 (OAuth2 Bearer).
    ApiV2,
}

impl Backend {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "cookie" => Some(Backend::Cookie),
            "api-v2" | "apiv2" | "v2" => Some(Backend::ApiV2),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Backend::Cookie => "cookie",
            Backend::ApiV2 => "api-v2",
        }
    }
}

/// v2 API roots.
pub const V2_API_ROOT: &str = "https://api.twitter.com/2";

/// Route one command to its backend. `(backend, reason)` — reason documents
/// *why* for `--explain`-style output and tests.
pub fn route(command: &str, requested: Backend) -> (Backend, &'static str) {
    // Cookie-only: official API has no equivalent surface.
    const COOKIE_ONLY: &[&str] = &["feed", "bookmarks", "list", "show", "headlines"];
    if COOKIE_ONLY.contains(&command) {
        return (
            Backend::Cookie,
            "official API v2 exposes no equivalent; cookie only",
        );
    }
    match requested {
        Backend::Cookie => (Backend::Cookie, "default backend"),
        Backend::ApiV2 => (Backend::ApiV2, "v2 covers this command"),
    }
}

/// v2 search URL with `--scope recent|all` (PR #31).
pub fn search_url(query: &str, scope: SearchScope, max: usize) -> String {
    let sc = match scope {
        SearchScope::Recent => "recent",
        SearchScope::All => "all",
    };
    format!(
        "{V2_API_ROOT}/tweets/search/{sc}?query={}&max_results={}",
        url_encode(query),
        max.clamp(10, 100)
    )
}

/// v2 search scope flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchScope {
    #[default]
    Recent,
    All,
}

impl SearchScope {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "recent" => Some(SearchScope::Recent),
            "all" => Some(SearchScope::All),
            _ => None,
        }
    }
}

/// v2 tweet lookup URL with `--reply-scope auto|recent|all`.
pub fn tweet_url(id: &str, reply_scope: ReplyScope) -> String {
    let base =
        format!("{V2_API_ROOT}/tweets/{id}?tweet.fields=created_at,public_metrics,author_id");
    match reply_scope {
        ReplyScope::Auto => base,
        ReplyScope::Recent => format!("{base}&expansions=referenced_tweets.id"),
        ReplyScope::All => format!(
            "{base}&expansions=referenced_tweets.id,attachments.media_keys&media.fields=url"
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplyScope {
    #[default]
    Auto,
    Recent,
    All,
}

impl ReplyScope {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "auto" => Some(ReplyScope::Auto),
            "recent" => Some(ReplyScope::Recent),
            "all" => Some(ReplyScope::All),
            _ => None,
        }
    }
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_only_ops_ignore_request() {
        for op in ["feed", "bookmarks", "list", "show", "headlines"] {
            assert_eq!(route(op, Backend::ApiV2).0, Backend::Cookie);
        }
        assert_eq!(route("search", Backend::ApiV2).0, Backend::ApiV2);
        assert_eq!(route("post", Backend::Cookie).0, Backend::Cookie);
    }

    #[test]
    fn urls_carry_scope_params() {
        assert!(search_url("rust", SearchScope::All, 20).contains("/search/all?"));
        assert!(search_url("rust", SearchScope::Recent, 200).contains("max_results=100"));
        assert!(tweet_url("1", ReplyScope::Auto).ends_with("author_id"));
        assert!(tweet_url("1", ReplyScope::All).contains("attachments.media_keys"));
        assert_eq!(Backend::parse("API-V2"), Some(Backend::ApiV2));
    }
}
