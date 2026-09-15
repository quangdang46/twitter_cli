//! P1-PARITY layer 1 (bead twitter_cli-5o3.3.11): byte-level fixture
//! deep-equal — each tests/fixtures/*.json deep-equals the Rust parser
//! output field-for-field (id/text/author/metrics/media/pagination).
//!
//! Fixtures are captured offline from the real Python parser via
//! tests/capture_fixtures.py (no network, no creds). The `input` payload
//! feeds the Rust parser; the result must equal `python_output` exactly
//! (as JSON values, so Option None == null on both sides).

use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
}

fn load_fixture(name: &str) -> serde_json::Value {
    let path = fixtures_dir().join(format!("{name}.json"));
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing fixture {path:?}; run tests/capture_fixtures.py"));
    serde_json::from_str(&raw).expect("fixture must be valid JSON")
}

fn rust_tweet_json(input: &serde_json::Value) -> serde_json::Value {
    match twr_model::parse_tweet_result(input, 0) {
        None => serde_json::Value::Null,
        Some(t) => serde_json::to_value(&t).expect("Tweet serializes"),
    }
}

fn rust_user_json(input: &serde_json::Value) -> serde_json::Value {
    match twr_model::parse_user_result(input) {
        None => serde_json::Value::Null,
        Some(u) => serde_json::to_value(&u).expect("UserProfile serializes"),
    }
}

/// Normalize both sides: drop JSON nulls recursively. Python's asdict
/// emits every Optional field as null; Rust skips them via
/// `skip_serializing_if`. Both mean "absent" — only real value
/// disagreements (wrong id/text/author/media/...) fail the test.
fn denull(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, denull(v)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(denull).collect())
        }
        other => other,
    }
}

fn assert_parity(name: &str, is_user: bool) {
    let doc = load_fixture(name);
    let input = &doc["input"];
    let expected = &doc["python_output"];
    let actual = if is_user {
        rust_user_json(input)
    } else {
        rust_tweet_json(input)
    };
    assert_eq!(
        denull(actual),
        denull(expected.clone()),
        "parity failure for {name}: Rust output != Python output"
    );
}

#[test]
fn parity_plain_tweet() {
    assert_parity("plain_tweet", false);
}

#[test]
fn parity_tombstone() {
    assert_parity("tombstone", false);
}

#[test]
fn parity_visibility_wrapped() {
    assert_parity("visibility_wrapped", false);
}

#[test]
fn parity_retweet() {
    assert_parity("retweet", false);
}

#[test]
fn parity_quote_tweet() {
    assert_parity("quote_tweet", false);
}

#[test]
fn parity_photo_media() {
    assert_parity("photo_media", false);
}

#[test]
fn parity_video_media() {
    assert_parity("video_media", false);
}

#[test]
fn parity_note_tweet() {
    assert_parity("note_tweet", false);
}

#[test]
fn parity_user() {
    assert_parity("user", true);
}

#[test]
fn parity_user_unavailable() {
    assert_parity("user_unavailable", true);
}
