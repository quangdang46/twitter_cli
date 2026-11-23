//! Opt-in timeline impression feedback (issue #20).
//!
//! X's web client reports which timeline entries were actually viewed by
//! POSTing event batches to `/i/api/1.1/flow/timeline.json` (form field
//! `log`). `twr` never fabricates these events: the caller observes real
//! visibility/dwell and supplies the batch. This module owns input
//! validation, local duplicate suppression, and the wire payload; the
//! runner in `main.rs` owns auth, policy, budgets, and idempotency.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Upper bound on events per invocation — matches the bounded-batch rule
/// in the issue; larger feeds must be split by the caller.
pub const MAX_BATCH: usize = 200;

/// How long a locally-seen event id suppresses resubmission.
const SEEN_TTL_SECS: u64 = 7 * 24 * 3600;
/// Hard cap on the seen-store so it cannot grow without bound.
const SEEN_MAX_IDS: usize = 10_000;

/// Parse the caller-supplied batch. Accepts either a top-level JSON array
/// of event objects or `{"events": [...]}`. Every entry must be a
/// non-empty JSON object — event semantics (impression vs. linger vs.
/// stream position, attribution tokens) belong to the caller and are
/// passed through verbatim.
pub fn parse_batch(input: &str) -> Result<Vec<serde_json::Value>, String> {
    let value: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("invalid JSON batch: {e}"))?;
    let events = match &value {
        serde_json::Value::Array(items) => items.clone(),
        serde_json::Value::Object(map) => match map.get("events") {
            Some(serde_json::Value::Array(items)) => items.clone(),
            _ => return Err("batch must be a JSON array or {\"events\": [...]}}".into()),
        },
        _ => return Err("batch must be a JSON array or {\"events\": [...]}".into()),
    };
    if events.is_empty() {
        return Err("batch is empty — nothing to submit".into());
    }
    if events.len() > MAX_BATCH {
        return Err(format!(
            "batch has {} events; max is {MAX_BATCH} — split the batch",
            events.len()
        ));
    }
    for (i, ev) in events.iter().enumerate() {
        match ev {
            serde_json::Value::Object(map) if !map.is_empty() => {}
            _ => return Err(format!("event {i} must be a non-empty JSON object")),
        }
    }
    Ok(events)
}

/// Local dedup key for one event: caller-supplied `id`/`event_id`
/// (string or number). Events without an id cannot be deduplicated
/// locally — server-side dedup is unverified, so callers that need
/// exactly-once should always set one.
pub fn event_id(ev: &serde_json::Value) -> Option<String> {
    for key in ["id", "event_id", "eventId"] {
        if let Some(v) = ev.get(key) {
            match v {
                serde_json::Value::String(s) if !s.is_empty() => return Some(s.clone()),
                serde_json::Value::Number(n) => return Some(n.to_string()),
                _ => {}
            }
        }
    }
    None
}

/// Split events into (new, skipped_duplicate_count) against the seen set.
pub fn partition_seen(
    events: &[serde_json::Value],
    seen: &HashSet<String>,
) -> (Vec<serde_json::Value>, usize) {
    let mut fresh = Vec::with_capacity(events.len());
    let mut skipped = 0usize;
    for ev in events {
        match event_id(ev) {
            Some(id) if seen.contains(&id) => skipped += 1,
            _ => fresh.push(ev.clone()),
        }
    }
    (fresh, skipped)
}

/// The `log` form field: a JSON array of the (deduplicated) events,
/// passed through verbatim so caller attribution survives.
pub fn log_payload(events: &[serde_json::Value]) -> String {
    serde_json::to_string(events).unwrap_or_else(|_| "[]".into())
}

/// `application/x-www-form-urlencoded` body for the flow endpoint. X's
/// client sends `debug=true&log=<json array>` — the debug flag is part of
/// the accepted request shape (same as the legacy jot/client_event
/// endpoint).
pub fn form_body(log: &str) -> Vec<u8> {
    let mut out = String::with_capacity(log.len() + 16);
    out.push_str("debug=true&log=");
    for b in log.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out.into_bytes()
}

/// Seen-store path under the twr state dir (`~/.twr/impressions_seen.json`).
pub fn seen_path() -> Option<PathBuf> {
    twr_core::budget::state_dir().map(|d| d.join("impressions_seen.json"))
}

/// Load the seen-id map, pruning entries older than [`SEEN_TTL_SECS`].
pub fn load_seen(path: &std::path::Path) -> HashMap<String, u64> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(map) = serde_json::from_str::<HashMap<String, u64>>(&raw) else {
        return HashMap::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    map.into_iter()
        .filter(|(_, ts)| now.saturating_sub(*ts) < SEEN_TTL_SECS)
        .collect()
}

/// Persist newly-seen ids, keeping the store bounded.
pub fn record_seen(path: &std::path::Path, ids: &[String], now: u64) {
    let mut map = load_seen(path);
    for id in ids {
        map.insert(id.clone(), now);
    }
    if map.len() > SEEN_MAX_IDS {
        // Drop the oldest entries first.
        let mut by_age: Vec<(String, u64)> = map.into_iter().collect();
        by_age.sort_by_key(|(_, ts)| *ts);
        by_age.truncate(SEEN_MAX_IDS);
        map = by_age.into_iter().collect();
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string(&map) {
        // tmp+rename: a crash mid-write must not truncate the seen store
        // into silent duplicate resubmissions.
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// Ids of the events that carry a dedup key (for record_seen after a
/// confirmed send).
pub fn sent_ids(events: &[serde_json::Value]) -> Vec<String> {
    events.iter().filter_map(event_id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_batch_accepts_array_and_events_wrapper() {
        let a = parse_batch(r#"[{"event":"impression","tweet_id":"1"}]"#).unwrap();
        assert_eq!(a.len(), 1);
        let b = parse_batch(r#"{"events":[{"event":"impression"}]}"#).unwrap();
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn parse_batch_rejects_bad_shapes() {
        assert!(parse_batch("{}").is_err());
        assert!(parse_batch("[]").is_err());
        assert!(parse_batch("\"x\"").is_err());
        assert!(parse_batch("[1]").is_err());
        assert!(parse_batch("[{}]").is_err());
        assert!(parse_batch("not json").is_err());
    }

    #[test]
    fn parse_batch_enforces_the_cap() {
        let big: Vec<serde_json::Value> = (0..=MAX_BATCH).map(|i| json!({"id": i})).collect();
        assert!(parse_batch(&serde_json::to_string(&big).unwrap()).is_err());
        let ok: Vec<serde_json::Value> = (0..MAX_BATCH).map(|i| json!({"id": i})).collect();
        assert!(parse_batch(&serde_json::to_string(&ok).unwrap()).is_ok());
    }

    #[test]
    fn event_id_reads_common_keys() {
        assert_eq!(event_id(&json!({"id":"a"})).as_deref(), Some("a"));
        assert_eq!(event_id(&json!({"event_id":"b"})).as_deref(), Some("b"));
        assert_eq!(event_id(&json!({"eventId":"c"})).as_deref(), Some("c"));
        assert_eq!(event_id(&json!({"id":42})).as_deref(), Some("42"));
        assert_eq!(event_id(&json!({"tweet_id":"1"})), None);
    }

    #[test]
    fn partition_seen_splits_duplicates() {
        let events = vec![
            json!({"id":"a","tweet_id":"1"}),
            json!({"id":"b","tweet_id":"2"}),
            json!({"tweet_id":"3"}), // no id → always fresh
        ];
        let seen: HashSet<String> = ["a".to_string()].into_iter().collect();
        let (fresh, skipped) = partition_seen(&events, &seen);
        assert_eq!(fresh.len(), 2);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn log_payload_preserves_events_verbatim() {
        let events =
            vec![json!({"event":"impression","tweet_id":"9","suggestion_details":{"token":"abc"}})];
        let log = log_payload(&events);
        let round: serde_json::Value = serde_json::from_str(&log).unwrap();
        assert_eq!(round[0]["suggestion_details"]["token"], "abc");
    }

    #[test]
    fn form_body_urlencodes_the_log_field() {
        let body = form_body("[{\"a\":1}]");
        let text = String::from_utf8(body).unwrap();
        assert!(text.starts_with("debug=true&log="));
        assert!(text.contains("%7B")); // '{' encoded
        assert!(!text.contains('{'));
    }

    #[test]
    fn seen_store_prunes_and_records() {
        let dir = std::env::temp_dir().join(format!("twr-impr-{}", std::process::id()));
        let path = dir.join("seen.json");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap();
        record_seen(&path, &["x".into(), "y".into()], now);
        let map = load_seen(&path);
        assert_eq!(map.len(), 2);
        // Stale entries are pruned on load.
        let stale = serde_json::to_string(&HashMap::from([("old".to_string(), 1u64)])).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, stale).unwrap();
        assert!(load_seen(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
