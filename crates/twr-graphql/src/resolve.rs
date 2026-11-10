//! 4-layer query-ID resolver (plan §7, bead hoi contract fix).
//!
//! Order per operation:
//! 1. `TWR_QID_<OP>` env pin (operator override, bypasses everything else).
//! 2. 24h disk cache (`~/.twr/query-ids.json`).
//! 3. `endpoints.yaml` `queryId` hand-patch (operator override of the
//!    shipped baseline — takes effect even when the cache is cold).
//! 4. Shipped baseline ([`crate::consts::FALLBACK_QUERY_IDS`]).
//!
//! EXTRA rotation IDs are NOT a resolution layer: the baseline stays
//! primary (the seeded alternates were live-verified but must never
//! shadow it). They are consulted ONLY after an HTTP 404, via
//! [`rotate_extra`], then the working ID is written back through layer 2
//! ([`crate::cache::put`]).
//!
//! Live rescrape ([`crate::scrape::rescrape`]) runs via `doctor --refresh`
//! (and its results are written back through layer 2). On an HTTP 404 the
//! caller invalidates the op's cached ID ([`invalidate_op`]), rotates to
//! the next EXTRA id, retries once, then surfaces exit 6 (contract-drift).

use crate::cache;
use crate::consts::fallback_query_id;
use crate::endpoints::EndpointsMap;
use std::collections::HashMap;

/// Known-alternate IDs per op, tried when the primary 404s. Seeded from
/// [`crate::consts::EXTRA_FALLBACK_IDS`]; `doctor --refresh` discoveries
/// may append entries. The resolver NEVER returns these eagerly — they are
/// rotation candidates for the post-404 retry only.
pub type ExtraRotation = HashMap<String, Vec<String>>;

/// Where a resolved ID came from. Not secret — safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QidSource {
    EnvPin,
    DiskCache,
    EndpointsYaml,
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
/// don't touch the process environment. `endpoints` is the parsed
/// `endpoints.yaml` map (empty when absent/malformed).
pub fn resolve(
    operation: &str,
    read_env: impl FnOnce(&str) -> Option<String>,
    disk: &cache::QueryIdCache,
    endpoints: &EndpointsMap,
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
    // Layer 3: endpoints.yaml hand-patch (beats the shipped baseline so an
    // operator fix takes effect without a rebuild; a fresh cache entry —
    // layer 2 — still wins because it is newer truth).
    if let Some(qid) = endpoints
        .get(operation)
        .and_then(|e| e.query_id.clone())
        .filter(|s| !s.is_empty())
    {
        return Some(ResolvedQid {
            query_id: qid,
            source: QidSource::EndpointsYaml,
        });
    }
    // Layer 4: shipped baseline. EXTRA rotation is deliberately NOT here —
    // see [`rotate_extra`]; the baseline stays primary.
    fallback_query_id(operation).map(|qid| ResolvedQid {
        query_id: qid.to_string(),
        source: QidSource::Baseline,
    })
}

/// Pop the next alternate after a 404: returns the head of the op's
/// rotation list (the caller retries with it once, then caches it on
/// success). Returns `None` when the list is empty — the caller then
/// surfaces exit 6.
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
    use crate::endpoints::EndpointEntry;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn no_endpoints() -> EndpointsMap {
        EndpointsMap::new()
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
        let mut endpoints = EndpointsMap::new();
        endpoints.insert(
            "SearchTimeline".into(),
            EndpointEntry {
                query_id: Some("yaml".into()),
                ..Default::default()
            },
        );
        let out = resolve(
            "SearchTimeline",
            |_| Some("pinned".into()),
            &disk,
            &endpoints,
            200,
        )
        .unwrap();
        assert_eq!(out.query_id, "pinned");
        assert_eq!(out.source, QidSource::EnvPin);
    }

    #[test]
    fn layer_order_is_cache_then_yaml_then_baseline() {
        let disk = cache::QueryIdCache::new();
        let endpoints = no_endpoints();
        let out = resolve("SearchTimeline", no_env, &disk, &endpoints, 0).unwrap();
        assert_eq!(out.source, QidSource::Baseline);
        assert_eq!(out.query_id, "VhUd6vHVmLBcw0uX-6jMLA");

        // endpoints.yaml patch beats baseline but loses to a fresh cache.
        let mut endpoints = EndpointsMap::new();
        endpoints.insert(
            "SearchTimeline".into(),
            EndpointEntry {
                query_id: Some("patched".into()),
                ..Default::default()
            },
        );
        let out = resolve("SearchTimeline", no_env, &disk, &endpoints, 0).unwrap();
        assert_eq!(out.source, QidSource::EndpointsYaml);
        assert_eq!(out.query_id, "patched");

        let mut disk = cache::QueryIdCache::new();
        disk.insert(
            "SearchTimeline".into(),
            CacheEntry {
                query_id: "fresh".into(),
                updated_at_secs: 0,
            },
        );
        let out = resolve("SearchTimeline", no_env, &disk, &endpoints, 1).unwrap();
        assert_eq!(out.source, QidSource::DiskCache);
        assert_eq!(out.query_id, "fresh");
    }

    #[test]
    fn extra_rotation_never_shadows_baseline_at_resolve_time() {
        // Bead hoi: seeded EXTRA ids must NOT be returned by resolve() —
        // they are post-404 rotation candidates only.
        let mut extra = ExtraRotation::new();
        extra.insert("ListMembers".into(), vec!["alt1".into(), "alt2".into()]);
        let out = resolve(
            "ListMembers",
            no_env,
            &cache::QueryIdCache::new(),
            &no_endpoints(),
            0,
        )
        .unwrap();
        assert_eq!(out.source, QidSource::Baseline);
        // Rotation pops in listed order, independent of resolve().
        assert_eq!(rotate_extra(&mut extra, "ListMembers"), Some("alt1".into()));
        assert_eq!(rotate_extra(&mut extra, "ListMembers"), Some("alt2".into()));
        assert_eq!(rotate_extra(&mut extra, "ListMembers"), None);
        assert_eq!(rotate_extra(&mut extra, "NoSuchOp"), None);
    }

    #[test]
    fn unknown_op_resolves_to_none() {
        let out = resolve(
            "NoSuchOp",
            no_env,
            &cache::QueryIdCache::new(),
            &no_endpoints(),
            0,
        );
        assert!(out.is_none());
    }
}
