//! Client-side idempotency state machine (plan §5.5, bead twitter_cli-5o3.4.2).
//!
//! Honest scope (reviewed and locked): **best-effort retry deduplication,
//! NOT server-side exactly-once**. X's unofficial GraphQL has no native
//! idempotency contract like Stripe's — this dedups only within local state.
//!
//! States: `prepared` (built, not sent) → `sent` (in flight) → `acknowledged`
//! (server success, result cached) | `unknown` (timeout after sending —
//! genuinely no way to know if the server received it).
//!
//! Rules:
//! - `unknown`: NEVER auto-retry — return an envelope reporting `unknown`
//!   with a suggestion to check manually.
//! - `acknowledged` + same key: return the cached result, do not resubmit.
//! - Entries live 24h in `~/.twr/idempotency.json`; expired entries are
//!   treated as absent (pruned on load).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// TTL for idempotency entries, in seconds (24h).
pub const IDEMPOTENCY_TTL_SECS: u64 = 24 * 3600;

/// Lifecycle state of one keyed write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteState {
    Prepared,
    Sent,
    Acknowledged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdempotencyEntry {
    pub state: WriteState,
    pub created_at_secs: u64,
    /// Cached server result, present only when acknowledged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

impl IdempotencyEntry {
    pub fn is_expired(&self, now_secs: u64) -> bool {
        now_secs.saturating_sub(self.created_at_secs) >= IDEMPOTENCY_TTL_SECS
    }
}

pub type IdempotencyStore = HashMap<String, IdempotencyEntry>;

/// Default store path: `~/.twr/idempotency.json`.
pub fn default_store_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".twr").join("idempotency.json"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Load the store, pruning expired entries. Missing/garbage file = empty.
pub fn load(path: &Path, now_secs: u64) -> IdempotencyStore {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let store: IdempotencyStore = serde_json::from_str(&raw).unwrap_or_default();
    store
        .into_iter()
        .filter(|(_, e)| !e.is_expired(now_secs))
        .collect()
}

/// Persist the whole store (creating parent dirs).
pub fn save(path: &Path, store: &IdempotencyStore) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string(store)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, raw)
}

/// Outcome of consulting the store before sending.
#[derive(Debug, Clone, PartialEq)]
pub enum PreCheck {
    /// No live entry — proceed; the caller must mark prepared→sent.
    Proceed,
    /// Acknowledged with this key — return the cached result, do NOT resubmit.
    ReplayCached(serde_json::Value),
    /// A previous attempt is in `sent` (crashed mid-flight?) or `unknown` —
    /// never auto-retry; surface to the operator.
    RefuseUnknown,
}

pub fn pre_check(store: &IdempotencyStore, key: &str) -> PreCheck {
    match store.get(key) {
        None => PreCheck::Proceed,
        Some(entry) => match entry.state {
            WriteState::Acknowledged => {
                PreCheck::ReplayCached(entry.result.clone().unwrap_or(serde_json::Value::Null))
            }
            WriteState::Prepared => PreCheck::Proceed,
            WriteState::Sent | WriteState::Unknown => PreCheck::RefuseUnknown,
        },
    }
}

/// Suggestion text for the `unknown` envelope.
pub const UNKNOWN_SUGGESTION: &str =
    "write state is unknown (timeout after sending) — check manually whether it posted before retrying with a NEW --idempotency-key";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledged_replays_sent_and_unknown_refuse() {
        let mut store = IdempotencyStore::new();
        assert_eq!(pre_check(&store, "k"), PreCheck::Proceed);
        store.insert(
            "done".into(),
            IdempotencyEntry {
                state: WriteState::Acknowledged,
                created_at_secs: 0,
                result: Some(serde_json::json!({"id": "1"})),
            },
        );
        assert_eq!(
            pre_check(&store, "done"),
            PreCheck::ReplayCached(serde_json::json!({"id": "1"}))
        );
        for (key, state) in [("s", WriteState::Sent), ("u", WriteState::Unknown)] {
            store.insert(
                key.into(),
                IdempotencyEntry {
                    state,
                    created_at_secs: 0,
                    result: None,
                },
            );
            assert_eq!(pre_check(&store, key), PreCheck::RefuseUnknown);
        }
    }

    #[test]
    fn expired_entries_prune_on_load() {
        let dir = std::env::temp_dir().join(format!("twr-idem-{}", std::process::id()));
        let path = dir.join("idempotency.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = IdempotencyStore::new();
        store.insert(
            "old".into(),
            IdempotencyEntry {
                state: WriteState::Acknowledged,
                created_at_secs: 100,
                result: None,
            },
        );
        save(&path, &store).unwrap();
        assert!(load(&path, 100 + IDEMPOTENCY_TTL_SECS + 1).is_empty());
        assert_eq!(load(&path, 101).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
