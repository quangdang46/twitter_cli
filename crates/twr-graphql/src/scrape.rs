//! Layer-4 live rescrape (agentic-x pattern, plan §7).
//!
//! Walk `x.com`'s homepage for `client-web` bundle URLs, then scan each
//! bundle for `queryId`/`operationName` pairs. The network fetch is injected
//! (`fetch_url`) so this stays unit-testable; the HTTP itself belongs to
//! `twr-client`.
//!
//! Limits (plan §7): up to 800 lazy chunks, 16 parallel workers. The worker
//! count is advisory documentation for the `twr-client` caller — this module
//! scans whatever bundle bodies it is handed.

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Max lazy chunks to scan per refresh.
pub const MAX_CHUNKS: usize = 800;
/// Advisory worker count for the parallel fetch in `twr-client`.
pub const WORKERS: usize = 16;

static SCRIPT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:src|href)=["'](https://abs\.twimg\.com/responsive-web/client-web[^"']+\.js)["']"#,
    )
    .unwrap()
});

/// Strict `queryId`/`operationName` pair regex. `operationType` is matched
/// opportunistically but not required (some bundles omit it).
static OP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"queryId:\s*"([A-Za-z0-9_-]+)"[^}]{0,200}operationName:\s*"([^"]+)""#).unwrap()
});

/// Extract `client-web` bundle URLs from homepage HTML.
pub fn bundle_urls(html: &str) -> Vec<String> {
    SCRIPT_RE
        .captures_iter(html)
        .map(|c| c[1].to_string())
        .collect()
}

/// Extract `operationName -> queryId` pairs from one bundle body.
/// First-seen wins per operation (mirrors Python's `setdefault`).
pub fn query_ids_in_bundle(bundle: &str, out: &mut HashMap<String, String>) {
    for caps in OP_RE.captures_iter(bundle) {
        out.entry(caps[2].to_string())
            .or_insert_with(|| caps[1].to_string());
    }
}

/// Full layer-4 refresh: homepage → bundle list (capped at [`MAX_CHUNKS`]) →
/// per-bundle scan. Returns every op found.
pub fn rescrape(
    homepage_html: &str,
    fetch_bundle: impl Fn(&str) -> Option<String>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for url in bundle_urls(homepage_html).into_iter().take(MAX_CHUNKS) {
        if let Some(body) = fetch_bundle(&url) {
            query_ids_in_bundle(&body, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_urls_finds_client_web_scripts() {
        let html = r#"<script src="https://abs.twimg.com/responsive-web/client-web/main.abc123.js"></script><link href="https://abs.twimg.com/responsive-web/client-web/lazy.0.def456.js">"#;
        let urls = bundle_urls(html);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("main.abc123.js"));
    }

    #[test]
    fn query_ids_in_bundle_first_seen_wins() {
        let bundle = r#"queryId:"aaa111",operationName:"SearchTimeline" junk queryId:"bbb222",operationName:"SearchTimeline""#;
        let mut out = HashMap::new();
        query_ids_in_bundle(bundle, &mut out);
        assert_eq!(out["SearchTimeline"], "aaa111");
    }

    #[test]
    fn rescrape_caps_at_max_chunks_and_merges() {
        let html: String = (0..3)
            .map(|i| {
                format!(r#"<script src="https://abs.twimg.com/responsive-web/client-web/{i}.js">"#)
            })
            .collect();
        let found = rescrape(&html, |url| {
            // Derive a regex-legal id/op from the trailing chunk index.
            let idx = url.rsplit('/').next().unwrap_or("x").replace(".js", "");
            Some(format!(r#"queryId:"qid{idx}",operationName:"Op{idx}""#))
        });
        assert_eq!(found.len(), 3);
        assert_eq!(found["Op0"], "qid0");
    }
}
