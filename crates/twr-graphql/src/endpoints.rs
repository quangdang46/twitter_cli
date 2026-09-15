//! `endpoints.yaml`: per-op `{queryId, features, toggles, rps, burst}` read
//! at RUNTIME (x-cli-vibe's data-driven pattern, plan §7), so operators can
//! hand-patch a stale ID without a rebuild.
//!
//! Merge rule: file entries override the shipped baseline per-op; ops absent
//! from the file keep baseline resolution. Malformed files are ignored
//! (empty map), never fatal.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EndpointEntry {
    #[serde(default, rename = "queryId")]
    pub query_id: Option<String>,
    #[serde(default)]
    pub features: HashMap<String, bool>,
    #[serde(default)]
    pub toggles: HashMap<String, bool>,
    #[serde(default)]
    pub rps: Option<f64>,
    #[serde(default)]
    pub burst: Option<u32>,
}

pub type EndpointsMap = HashMap<String, EndpointEntry>;

/// Parse an `endpoints.yaml` document. `None` = missing file; `Some(Err)` =
/// unparseable (caller logs and treats as empty).
pub fn load_yaml(raw: Option<&str>) -> EndpointsMap {
    match raw {
        None => HashMap::new(),
        Some(text) => serde_yaml::from_str(text).unwrap_or_default(),
    }
}

/// Effective query ID for one op: file override wins, else `fallback`.
pub fn effective_query_id(
    endpoints: &EndpointsMap,
    operation: &str,
    fallback: Option<&str>,
) -> Option<String> {
    endpoints
        .get(operation)
        .and_then(|e| e.query_id.clone())
        .filter(|s| !s.is_empty())
        .or_else(|| fallback.map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
SearchTimeline:\n  queryId: patched-qid-123\n  rps: 0.5\n  burst: 2\nFollowing:\n  rps: 1.0\n";

    #[test]
    fn file_override_wins_over_fallback() {
        let map = load_yaml(Some(SAMPLE));
        assert_eq!(
            effective_query_id(&map, "SearchTimeline", Some("baseline")),
            Some("patched-qid-123".into())
        );
        assert_eq!(
            effective_query_id(&map, "Following", Some("baseline-f")),
            Some("baseline-f".into())
        );
        assert_eq!(map["Following"].rps, Some(1.0));
    }

    #[test]
    fn missing_and_garbage_yaml_yield_empty_map() {
        assert!(load_yaml(None).is_empty());
        assert!(load_yaml(Some("{{{not yaml")).is_empty());
    }
}
