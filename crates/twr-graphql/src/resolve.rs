//! 4-layer query-ID resolver (plan §7).
//!
//! Order per operation:
//! 1. `TWR_QID_<OP>` env pin (operator override, bypasses everything else).
//! 2. 24h disk cache (`~/.twr/query-ids.json`).
//! 3. EXTRA rotation list (known-alternate IDs per op, tried on 404).
//! 4. Shipped baseline ([`crate::consts::FALLBACK_QUERY_IDS`]).
//!
//! Live rescrape ([`crate::scrape::rescrape`]) is NOT a resolution layer —
//! it runs only via `doctor --refresh` or after a 404 invalidates the cached
//! ID, and its results are written back through layer 2.
//!
//! On an HTTP 404 from the endpoint itself: invalidate that op's cached ID,
//! trigger one refresh, retry once, then surface exit 6 (contract-drift) if
//! still failing. That retry policy lives in `twr-client`; this module
//! exposes the primitives ([`invalidate_op`]) it needs.

use crate::cache;
use crate::consts::fallback_query_id;
use std::collections::HashMap;

/// Known-alternate IDs per op, tried when the primary 404s. Seeded empty —
/// operators append entries via `endpoints.yaml` or `doctor --refresh`
/// discoveries; the resolver treats it as an ordered rotation list.
pub type ExtraRotation = HashMap<String, Vec<String>>;

/// Where a resolved ID came from. Not secret — safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QidSource {
    EnvPin,
    DiskCache,
    ExtraRotation,
    Baseline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedQid {
    pub query_id: String,
    pub source: QidSource,
}

/// Env var name pinning one op: `TWR_QID_<OP>` (e.g. `TWR_QID_SearchTimeline`).
pub fn env_pin_name(operation: &str) -> String {
    format!("TWR_QID_{operation}")
}

/// Resolve one op's ID through layers 1→4. `read_env` is injected so tests
/// don't touch the process environment.
pub fn resolve(
    operation: &str,
    read_env: impl FnOnce(&str) -> Option<String>,
    disk: &cache::QueryIdCache,
    extra: &ExtraRotation,
    now_secs: u64,
) -> Option<ResolvedQid> {
    // Layer 1: operator pin.
    if let Some(pinned) = read_env(&env_pin_name(operation)).filter(|v| !v.is_empty()) {
        return Some(ResolvedQid {
            query_id: pinned,
            source: QidSource::EnvPin,
        });
    }
    // Layer 2: disk cache.
    if let Some(qid) = cache::get_fresh(disk, operation, now_secs) {
        return Some(ResolvedQid {
            query_id: qid,
            source: QidSource::DiskCache,
        });
    }
    // Layer 3: extra rotation (head of the list).
    if let Some(qid) = extra.get(operation).and_then(|v| v.first()) {
        return Some(ResolvedQid {
            query_id: qid.clone(),
            source: QidSource::ExtraRotation,
        });
    }
    // Layer 4: shipped baseline.
    fallback_query_id(operation).map(|qid| ResolvedQid {
        query_id: qid.to_string(),
        source: QidSource::Baseline,
    })
}

/// Rotate to the next alternate after a 404: drop the head of the rotation
/// list and return it (the caller retries with it once). Returns `None` when
/// the list is empty.
pub fn rotate_extra(extra: &mut ExtraRotation, operation: &str) -> Option<String> {
    let list = extra.get_mut(operation)?;
    if list.is_empty() {
        return None;
    }
    Some(list.remove(0))
}

/// Drop one op's disk-cache entry after an endpoint 404.
pub fn invalidate_op(cache_path: &std::path::Path, operation: &str) {
    cache::invalidate(cache_path, operation);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheEntry;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn env_pin_beats_everything() {
        let mut disk = cache::QueryIdCache::new();
        disk.insert(
            "SearchTimeline".into(),
            CacheEntry {
                query_id: "cached".into(),
                updated_at_secs: 100,
            },
        );
        let mut extra = ExtraRotation::new();
        extra.insert("SearchTimeline".into(), vec!["rot".into()]);
        let out = resolve(
            "SearchTimeline",
            |_| Some("pinned".into()),
            &disk,
            &extra,
            200,
        )
        .unwrap();
        assert_eq!(out.query_id, "pinned");
        assert_eq!(out.source, QidSource::EnvPin);
    }

    #[test]
    fn layer_order_is_cache_then_rotation_then_baseline() {
        let disk = cache::QueryIdCache::new();
        let extra = ExtraRotation::new();
        let out = resolve("SearchTimeline", no_env, &disk, &extra, 0).unwrap();
        assert_eq!(out.source, QidSource::Baseline);
        assert_eq!(out.query_id, "VhUd6vHVmLBcw0uX-6jMLA");
    }

    #[test]
    fn unknown_op_resolves_to_none() {
        let out = resolve(
            "NoSuchOp",
            no_env,
            &cache::QueryIdCache::new(),
            &ExtraRotation::new(),
            0,
        );
        assert!(out.is_none());
    }

    #[test]
    fn rotate_extra_pops_head() {
        let mut extra = ExtraRotation::new();
        extra.insert("Op".into(), vec!["a".into(), "b".into()]);
        assert_eq!(rotate_extra(&mut extra, "Op"), Some("a".into()));
        assert_eq!(rotate_extra(&mut extra, "Op"), Some("b".into()));
        assert_eq!(rotate_extra(&mut extra, "Op"), None);
    }
}
