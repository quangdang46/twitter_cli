//! 1-hour file cache for the derived transaction ingredients.
//!
//! The ingredients (verification key bytes + animation key) come from a live
//! fetch of `x.com`'s home page + `ondemand.s` chunk — an HTTP round trip that
//! belongs to `twr-client`, not here. Once derived they stay valid long enough
//! that re-deriving per request would be pure waste, so this module persists
//! them to `~/.twr/transaction_cache.json` with a 1h TTL (plan §7).
//!
//! The cached value is the *ingredient pair*, never a mintable transaction id:
//! ids are single-use and must be freshly generated per request via
//! [`ClientTransactionV1::generate`](crate::ClientTransactionV1::generate).
//! Replaying a cached id 404s.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// TTL for cached ingredients, in seconds (plan §7: 1h).
pub const CACHE_TTL_SECS: u64 = 3600;

/// On-disk shape of `~/.twr/transaction_cache.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedIngredients {
    /// Base64-encoded verification key bytes.
    pub key_b64: String,
    pub animation_key: String,
    /// Unix seconds when the ingredients were derived.
    pub derived_at_secs: u64,
}

impl CachedIngredients {
    pub fn new(key_bytes: &[u8], animation_key: &str, now_secs: u64) -> Self {
        Self {
            key_b64: STANDARD.encode(key_bytes),
            animation_key: animation_key.to_string(),
            derived_at_secs: now_secs,
        }
    }

    pub fn key_bytes(&self) -> Result<Vec<u8>, String> {
        STANDARD
            .decode(&self.key_b64)
            .map_err(|e| format!("transaction cache key_b64 is not valid base64: {e}"))
    }

    /// True when the entry is younger than [`CACHE_TTL_SECS`].
    pub fn is_fresh(&self, now_secs: u64) -> bool {
        now_secs.saturating_sub(self.derived_at_secs) < CACHE_TTL_SECS
    }
}

/// Default cache location: `~/.twr/transaction_cache.json`.
pub fn default_cache_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".twr").join("transaction_cache.json"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Current unix time in whole seconds. Split out so tests can pin time.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Load cached ingredients if the file exists, parses, and is still fresh.
/// Returns `None` on any failure (missing file, bad JSON, stale entry) — a
/// cache miss just means the caller re-derives from a live fetch.
pub fn load_fresh(path: &std::path::Path, now: u64) -> Option<CachedIngredients> {
    let raw = std::fs::read_to_string(path).ok()?;
    let cached: CachedIngredients = serde_json::from_str(&raw).ok()?;
    if cached.is_fresh(now) {
        Some(cached)
    } else {
        None
    }
}

/// Persist ingredients, creating parent directories as needed. Errors are
/// returned to the caller; caching is best-effort so callers may ignore them.
pub fn store(path: &std::path::Path, cached: &CachedIngredients) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string(cached)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CachedIngredients {
        CachedIngredients::new(&[1u8, 2, 3], "abc123", 1_800_000_000)
    }

    #[test]
    fn fresh_within_ttl_stale_after() {
        let c = sample();
        assert!(c.is_fresh(1_800_000_000));
        assert!(c.is_fresh(1_800_000_000 + CACHE_TTL_SECS - 1));
        assert!(!c.is_fresh(1_800_000_000 + CACHE_TTL_SECS));
        assert!(!c.is_fresh(1_800_000_000 + 7200));
    }

    #[test]
    fn key_bytes_round_trip() {
        let c = sample();
        assert_eq!(c.key_bytes().unwrap(), vec![1u8, 2, 3]);
    }

    #[test]
    fn load_fresh_hits_fresh_entry_and_misses_stale_or_garbage() {
        let dir = std::env::temp_dir().join(format!("twr-tx-cache-test-{}", std::process::id()));
        let path = dir.join("transaction_cache.json");
        let _ = std::fs::remove_dir_all(&dir);

        // Missing file -> None.
        assert!(load_fresh(&path, 1_800_000_000).is_none());

        // Fresh entry -> hit.
        store(&path, &sample()).unwrap();
        let hit = load_fresh(&path, 1_800_000_000 + 10).unwrap();
        assert_eq!(hit.animation_key, "abc123");

        // Stale entry -> miss.
        assert!(load_fresh(&path, 1_800_000_000 + 7200).is_none());

        // Garbage bytes -> miss, not a panic.
        std::fs::write(&path, "not json{{").unwrap();
        assert!(load_fresh(&path, 1_800_000_000).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn store_creates_parent_dirs() {
        let dir = std::env::temp_dir().join(format!("twr-tx-cache-nest-{}", std::process::id()));
        let path = dir.join("a").join("b").join("transaction_cache.json");
        let _ = std::fs::remove_dir_all(&dir);
        store(&path, &sample()).unwrap();
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_cache_path_ends_in_twr_transaction_cache_json() {
        let p = default_cache_path().unwrap();
        assert!(
            p.ends_with(".twr/transaction_cache.json")
                || p.ends_with(".twr\\transaction_cache.json")
        );
    }
}
