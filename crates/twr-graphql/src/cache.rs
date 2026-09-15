//! Layer-2 disk cache: `~/.twr/query-ids.json` with a 24h TTL (plan §7).
//!
//! Shape: `{op: {query_id, updated_at_secs}}`. Stale or missing entries are a
//! miss, never an error — the resolver just falls through to the next layer.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// TTL for cached query IDs, in seconds (plan §7: 24h).
pub const CACHE_TTL_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub query_id: String,
    pub updated_at_secs: u64,
}

impl CacheEntry {
    pub fn is_fresh(&self, now_secs: u64) -> bool {
        now_secs.saturating_sub(self.updated_at_secs) < CACHE_TTL_SECS
    }
}

/// Default cache location: `~/.twr/query-ids.json`.
pub fn default_cache_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".twr").join("query-ids.json"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub type QueryIdCache = HashMap<String, CacheEntry>;

/// Load the whole cache file. `None` = missing/unparseable (a miss, not an
/// error).
pub fn load(path: &Path) -> Option<QueryIdCache> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Fresh ID for one op, or `None` on miss/stale.
pub fn get_fresh(cache: &QueryIdCache, operation: &str, now_secs: u64) -> Option<String> {
    cache
        .get(operation)
        .filter(|e| e.is_fresh(now_secs))
        .map(|e| e.query_id.clone())
}

/// Insert/update one op and persist. Best-effort: errors are returned so the
/// caller may ignore them.
pub fn put(path: &Path, operation: &str, query_id: &str, now_secs: u64) -> std::io::Result<()> {
    let mut cache = load(path).unwrap_or_default();
    cache.insert(
        operation.to_string(),
        CacheEntry {
            query_id: query_id.to_string(),
            updated_at_secs: now_secs,
        },
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string(&cache)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, raw)
}

/// Drop one op (called on endpoint-404 before the single refresh+retry).
pub fn invalidate(path: &Path, operation: &str) {
    if let Some(mut cache) = load(path) {
        if cache.remove(operation).is_some() {
            let _ = std::fs::write(path, serde_json::to_string(&cache).unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        std::env::temp_dir().join(format!("twr-qid-cache-{}", std::process::id()))
    }

    #[test]
    fn fresh_within_24h_stale_after() {
        let e = CacheEntry {
            query_id: "abc".into(),
            updated_at_secs: 1_000_000,
        };
        assert!(e.is_fresh(1_000_000 + CACHE_TTL_SECS - 1));
        assert!(!e.is_fresh(1_000_000 + CACHE_TTL_SECS));
    }

    #[test]
    fn put_get_invalidate_round_trip() {
        let dir = tmp();
        let path = dir.join("query-ids.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&path).is_none());
        put(&path, "SearchTimeline", "qid1", 1_000_000).unwrap();
        let cache = load(&path).unwrap();
        assert_eq!(
            get_fresh(&cache, "SearchTimeline", 1_000_100),
            Some("qid1".into())
        );
        assert_eq!(
            get_fresh(&cache, "SearchTimeline", 1_000_000 + 100_000),
            None
        );
        invalidate(&path, "SearchTimeline");
        assert!(!load(&path).unwrap().contains_key("SearchTimeline"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
