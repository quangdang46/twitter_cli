//! `twr-tx` — derives X's per-request `x-client-transaction-id` header.
//!
//! **This module is fragile by design and by nature.** X can change this
//! obfuscated, reverse-engineered client-side algorithm on any deploy — a 404
//! with an empty body from a gated op is what the wall looks like, and the fix
//! is to re-port the current public implementation, not to debug this file
//! (see COMPREHENSIVEPLANFORTWITTERCLI.md §7 and the P0 spike bead
//! `twitter_cli-5o3.2.3`).
//!
//! Ported from `iSarabjitDhiman/XClientTransaction` (MIT) via
//! `tmp/_research/agentic-x/src/agentic_x/transaction.py`, which this crate's
//! test vectors were captured from directly (see `tests` module below) —
//! true cross-language parity, not just internal self-consistency.
//!
//! Design (COMPREHENSIVEPLANFORTWITTERCLI.md §3/§7): the [`RequestProof`]
//! trait is the stable interface; [`ClientTransactionV1`] is the current (and
//! only) implementation. If X changes or drops the requirement, only the impl
//! swaps — call sites never change. The header is single-use: a fresh id must
//! be minted per request, and it is attached ONLY to [`GATED_OPS`] — every
//! other GraphQL operation must NOT send it.

mod frames;
mod math;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use thiserror::Error;

pub use frames::extract_frame_paths;

/// The only ops that need the header, live-probed 2026-07-20 per the
/// agentic-x reference. Everything else answers 200 without it and must not
/// be sent one — attaching it to an ungated op is one more thing that can
/// break for no benefit.
pub const GATED_OPS: [&str; 3] = ["SearchTimeline", "UserTweetsAndReplies", "Followers"];

const ADDITIONAL_RANDOM_NUMBER: u8 = 3;
const DEFAULT_KEYWORD: &str = "obfiowerehiring";
const TOTAL_TIME: f64 = 4096.0;
/// X's own epoch offset for the timestamp bytes (2023-05-01T07:00:00Z).
const EPOCH_OFFSET_SECONDS: f64 = 1_682_924_400.0;

static VERIFICATION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<meta[^>]+name=["']twitter-site-verification["'][^>]+content=["']([^"']+)["']"#)
        .unwrap()
});
static INDICES_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(\w\[(\d{1,2})\],\s*16\)").unwrap());
static NON_DIGIT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^0-9]+").unwrap());

#[derive(Debug, Error)]
pub enum TxError {
    #[error("{0}")]
    Derivation(String),
}

/// A pluggable per-request "proof" mechanism — today X's transaction-ID
/// scheme, potentially something else entirely if X changes its anti-bot
/// story. `prepare` returns `None` for any operation that shouldn't carry a
/// proof header at all (see [`GATED_OPS`]).
pub trait RequestProof: Send + Sync {
    /// `operation` is the GraphQL operation name (e.g. `"SearchTimeline"`);
    /// `method`/`path` are the HTTP method and URL path the header signs.
    fn prepare(&self, operation: &str, method: &str, path: &str) -> Option<String>;
}

/// Extract the verification key and the four animation frames from x.com's
/// home-page HTML (the two "ingredients" `ClientTransactionV1` needs, besides
/// the on-demand-chunk key-byte indices which come from a second fetch).
pub fn extract_verification_key(html: &str) -> Result<Vec<u8>, TxError> {
    let caps = VERIFICATION_RE
        .captures(html)
        .ok_or_else(|| TxError::Derivation("no twitter-site-verification meta tag found".into()))?;
    STANDARD
        .decode(&caps[1])
        .map_err(|e| TxError::Derivation(format!("verification key is not valid base64: {e}")))
}

/// Extract the `(x[N], 16)`-shaped key-byte indices from the `ondemand.s`
/// chunk's JS source. The first index is the row-selector byte; the rest feed
/// the frame-time product.
pub fn extract_key_byte_indices(ondemand_js: &str) -> Vec<usize> {
    INDICES_RE
        .captures_iter(ondemand_js)
        .map(|c| c[1].parse().unwrap())
        .collect()
}

/// Derive the animation key from the page's four SVG frames. Pure function —
/// no network — so it's independently unit-testable against a synthetic page.
pub fn compute_animation_key(
    key_bytes: &[u8],
    frames: &std::collections::BTreeMap<u32, Vec<String>>,
    row_index_key: usize,
    byte_indices: &[usize],
) -> Result<String, TxError> {
    let frame_idx = (key_bytes[5] as u32) % 4;
    let frame_paths = frames.get(&frame_idx).cloned().unwrap_or_default();
    if frame_paths.len() < 2 {
        return Err(TxError::Derivation(format!(
            "loading-x-anim frame {} has {} paths, expected >= 2",
            frame_idx,
            frame_paths.len()
        )));
    }

    let d = &frame_paths[1];
    let sliced: String = d.chars().skip(9).collect();
    let rows: Vec<Vec<i64>> = sliced
        .split('C')
        .map(|segment| {
            let cleaned = NON_DIGIT_RE.replace_all(segment, " ");
            cleaned
                .split_whitespace()
                .map(|n| n.parse::<i64>().unwrap())
                .collect()
        })
        .collect();

    let row_index = (key_bytes[row_index_key] as usize) % 16;
    let frame_time_product: i64 = byte_indices
        .iter()
        .map(|&i| (key_bytes[i] as i64) % 16)
        .product();
    let frame_time = math::js_round(frame_time_product as f64 / 10.0) * 10.0;

    Ok(math::animate(&rows[row_index], frame_time / TOTAL_TIME))
}

/// Compute one transaction id given fully-resolved ingredients. Split out
/// from [`ClientTransactionV1::generate`] so tests can pin `now_seconds` and
/// `noise` exactly like the Python suite's monkeypatched `time.time()`/
/// `random.randint()`.
pub fn generate_txid(
    key_bytes: &[u8],
    animation_key: &str,
    method: &str,
    path: &str,
    now_seconds: f64,
    noise: u8,
) -> String {
    let time_now = ((now_seconds * 1000.0 - EPOCH_OFFSET_SECONDS * 1000.0) / 1000.0).floor() as i64;
    let time_bytes: [u8; 4] = [
        (time_now & 0xFF) as u8,
        ((time_now >> 8) & 0xFF) as u8,
        ((time_now >> 16) & 0xFF) as u8,
        ((time_now >> 24) & 0xFF) as u8,
    ];
    let message = format!("{method}!{path}!{time_now}{DEFAULT_KEYWORD}{animation_key}");
    let digest = Sha256::digest(message.as_bytes());

    let mut payload: Vec<u8> = Vec::with_capacity(key_bytes.len() + 4 + 16 + 1);
    payload.extend_from_slice(key_bytes);
    payload.extend_from_slice(&time_bytes);
    payload.extend_from_slice(&digest[..16]);
    payload.push(ADDITIONAL_RANDOM_NUMBER);

    let mut out = Vec::with_capacity(payload.len() + 1);
    out.push(noise);
    out.extend(payload.into_iter().map(|b| b ^ noise));

    STANDARD.encode(out).trim_end_matches('=').to_string()
}

/// Mints fresh transaction ids for one session. The three page-derived
/// ingredients (key bytes, animation key) are resolved once (by the caller,
/// via [`extract_verification_key`]/[`compute_animation_key`] against a real
/// fetch of `x.com/home` — that HTTP call belongs to `twr-client`, not here)
/// and cached for this object's lifetime; each id is then computed locally.
pub struct ClientTransactionV1 {
    key_bytes: Vec<u8>,
    animation_key: String,
}

impl ClientTransactionV1 {
    pub fn new(key_bytes: Vec<u8>, animation_key: String) -> Self {
        Self {
            key_bytes,
            animation_key,
        }
    }

    /// Return a fresh id for one request. Never cache or replay the result —
    /// replaying a real captured id 404s, because X's own client already
    /// spent it.
    pub fn generate(&self, method: &str, path: &str) -> String {
        let now_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_secs_f64();
        let noise: u8 = rand::random();
        generate_txid(
            &self.key_bytes,
            &self.animation_key,
            method,
            path,
            now_seconds,
            noise,
        )
    }
}

impl RequestProof for ClientTransactionV1 {
    fn prepare(&self, operation: &str, method: &str, path: &str) -> Option<String> {
        if GATED_OPS.contains(&operation) {
            Some(self.generate(method, path))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // Mirrors tmp/_research/agentic-x/tests/test_transaction.py exactly: same
    // synthetic page, same key bytes. key_bytes = 0..47, so:
    //   key_bytes[5] % 4  == 1  -> frame 1 is the one read
    //   key_bytes[24] % 16 == 8 -> row 8 of that frame's second path
    fn key_bytes() -> Vec<u8> {
        (0u8..48).collect()
    }
    const ROW_INDEX_KEY: usize = 24;
    const BYTE_INDICES: [usize; 3] = [29, 31, 22];

    fn path_d() -> String {
        let segments: Vec<String> = (0..12)
            .map(|row| {
                (0..11)
                    .map(|n| ((row * 11 + n) % 256).to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        format!("M0 0 C 0 {}", segments.join("C"))
    }

    fn page() -> String {
        let key_b64 = STANDARD.encode(key_bytes());
        let d = path_d();
        let frames: String = (0..4)
            .map(|i| {
                format!(
                    r#"<svg id="loading-x-anim-{i}"><g><path d="M1 1"/><path d="{d}"/></g></svg>"#
                )
            })
            .collect();
        format!(
            r#"<html><head><meta name="twitter-site-verification" content="{key_b64}"/></head><body>{frames}<script>e={{,59924:"ondemand.s",59924:"deadbeef"}}</script></body></html>"#
        )
    }

    fn ondemand_js() -> &'static str {
        r#"function d(a){return parseInt(a[24], 16)+parseInt(a[29], 16)+parseInt(a[31], 16)+parseInt(a[22], 16)}"#
    }

    #[test]
    fn extract_verification_key_decodes_the_meta_tag() {
        let decoded = extract_verification_key(&page()).unwrap();
        assert_eq!(decoded, key_bytes());
    }

    #[test]
    fn indices_regex_finds_key_byte_indices() {
        let indices = extract_key_byte_indices(ondemand_js());
        assert_eq!(indices, vec![ROW_INDEX_KEY, 29, 31, 22]);
    }

    #[test]
    fn compute_animation_key_is_deterministic() {
        let frames = extract_frame_paths(&page());
        let first =
            compute_animation_key(&key_bytes(), &frames, ROW_INDEX_KEY, &BYTE_INDICES).unwrap();
        let second =
            compute_animation_key(&key_bytes(), &frames, ROW_INDEX_KEY, &BYTE_INDICES).unwrap();
        assert_eq!(first, second);
        assert!(!first.is_empty());
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn compute_animation_key_rejects_a_frame_without_two_paths() {
        let mut frames = BTreeMap::new();
        frames.insert(1u32, vec!["only-one".to_string()]);
        let err =
            compute_animation_key(&key_bytes(), &frames, ROW_INDEX_KEY, &BYTE_INDICES).unwrap_err();
        assert!(matches!(err, TxError::Derivation(msg) if msg.contains("expected >= 2")));
    }

    /// TRUE CROSS-LANGUAGE PARITY: this exact value was captured by running
    /// `tmp/_research/agentic-x`'s `transaction.compute_animation_key` with
    /// these exact inputs (see the P0 spike bead's close notes for the
    /// capture command). If this ever fails, the Rust port has diverged from
    /// the Python reference — re-check every function in `math.rs` against
    /// `transaction.py`, do not just update the expected string.
    #[test]
    fn compute_animation_key_matches_python_reference_exactly() {
        let frames = extract_frame_paths(&page());
        let key =
            compute_animation_key(&key_bytes(), &frames, ROW_INDEX_KEY, &BYTE_INDICES).unwrap();
        assert_eq!(
            key,
            "58595a0ee147ae147ae1805c28f5c28f5c2805c28f5c28f5c280ee147ae147ae1800"
        );
    }

    #[test]
    fn generate_is_stable_for_a_fixed_time_and_noise() {
        let kb = key_bytes();
        let first = generate_txid(
            &kb,
            "abc123",
            "GET",
            "/i/api/graphql/QID/SearchTimeline",
            1_800_000_000.0,
            7,
        );
        let second = generate_txid(
            &kb,
            "abc123",
            "GET",
            "/i/api/graphql/QID/SearchTimeline",
            1_800_000_000.0,
            7,
        );
        assert_eq!(first, second);
    }

    #[test]
    fn generate_signs_the_path_so_two_ops_differ() {
        let kb = key_bytes();
        let search = generate_txid(
            &kb,
            "abc123",
            "GET",
            "/i/api/graphql/QID/SearchTimeline",
            1_800_000_000.0,
            7,
        );
        let replies = generate_txid(
            &kb,
            "abc123",
            "GET",
            "/i/api/graphql/QID/UserTweetsAndReplies",
            1_800_000_000.0,
            7,
        );
        assert_ne!(search, replies);
    }

    #[test]
    fn generate_signs_the_method() {
        let kb = key_bytes();
        let get = generate_txid(&kb, "abc123", "GET", "/p", 1_800_000_000.0, 7);
        let post = generate_txid(&kb, "abc123", "POST", "/p", 1_800_000_000.0, 7);
        assert_ne!(get, post);
    }

    #[test]
    fn generate_varies_with_the_random_noise_byte() {
        let kb = key_bytes();
        let a = generate_txid(&kb, "abc123", "GET", "/p", 1_800_000_000.0, 7);
        let b = generate_txid(&kb, "abc123", "GET", "/p", 1_800_000_000.0, 200);
        assert_ne!(a, b);
    }

    #[test]
    fn generate_produces_an_unpadded_base64_id_of_length_94() {
        let kb = key_bytes();
        let txid = generate_txid(
            &kb,
            "abc123",
            "GET",
            "/i/api/graphql/QID/SearchTimeline",
            1_800_000_000.0,
            7,
        );
        assert!(!txid.ends_with('='));
        assert_eq!(txid.len(), 94);
    }

    #[test]
    fn gated_ops_are_exactly_the_three_probed_walls() {
        assert_eq!(
            GATED_OPS,
            ["SearchTimeline", "UserTweetsAndReplies", "Followers"]
        );
    }

    #[test]
    fn ungated_ops_get_no_proof() {
        let ct = ClientTransactionV1::new(key_bytes(), "abc123".to_string());
        for op in ["UserTweets", "TweetDetail", "HomeTimeline", "Following"] {
            assert!(ct.prepare(op, "GET", "/x").is_none());
        }
    }

    /// TRUE CROSS-LANGUAGE PARITY vectors, captured from the Python reference
    /// with `time.time()` and `random.randint()` monkeypatched exactly as
    /// shown (see agentic-x's `_prepared()` test helper for the equivalent).
    #[test]
    fn generate_matches_python_reference_exactly() {
        let kb = key_bytes();
        assert_eq!(
            generate_txid(&kb, "abc123", "GET", "/i/api/graphql/QID/SearchTimeline", 1_800_000_000.0, 7),
            "BwcGBQQDAgEADw4NDAsKCQgXFhUUExIREB8eHRwbGhkYJyYlJCMiISAvLi0sKyopKJdp/QECH3xvrKfFl5rnG0wATOGwBA"
        );
        assert_eq!(
            generate_txid(&kb, "abc123", "GET", "/i/api/graphql/QID/UserTweetsAndReplies", 1_800_000_000.0, 7),
            "BwcGBQQDAgEADw4NDAsKCQgXFhUUExIREB8eHRwbGhkYJyYlJCMiISAvLi0sKyopKJdp/QGO3+Qu3yTjLVhEURYj5JmKBA"
        );
        assert_eq!(
            generate_txid(&kb, "abc123", "GET", "/i/api/graphql/QID/SearchTimeline", 1_800_000_000.0, 200),
            "yMjJysvMzc7PwMHCw8TFxsfY2drb3N3e39DR0tPU1dbX6Onq6+zt7u/g4eLj5OXm51imMs7N0LOgY2gKWFUo1IPPgy5/yw"
        );
        // Uses the REAL animation key derived from the synthetic page above,
        // not the fixed "abc123" stub -- exercises the full pipeline end to end.
        let frames = extract_frame_paths(&page());
        let real_key = compute_animation_key(&kb, &frames, ROW_INDEX_KEY, &BYTE_INDICES).unwrap();
        assert_eq!(
            generate_txid(&kb, &real_key, "GET", "/i/api/graphql/QID/SearchTimeline", 1_800_000_000.0, 7),
            "BwcGBQQDAgEADw4NDAsKCQgXFhUUExIREB8eHRwbGhkYJyYlJCMiISAvLi0sKyopKJdp/QFFLX+NNhejZwSMdTbfSU04BA"
        );
    }
}
