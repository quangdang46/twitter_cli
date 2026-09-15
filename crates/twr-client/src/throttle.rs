//! Per-endpoint token bucket (x-cli-vibe pattern, plan §7).
//!
//! Rates come from `endpoints.yaml`'s per-op `{rps, burst}` fields at runtime.
//! Pure-logic state machine — the sleeping/waiting belongs to the caller, so
//! this stays unit-testable with an injected clock.

use std::collections::HashMap;

/// Rate config for one endpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucketConfig {
    pub rps: f64,
    pub burst: f64,
}

impl Default for BucketConfig {
    fn default() -> Self {
        Self {
            rps: 1.0,
            burst: 4.0,
        }
    }
}

#[derive(Debug, Clone)]
struct Bucket {
    config: BucketConfig,
    tokens: f64,
    last_secs: f64,
}

/// Token-bucket throttle keyed by operation name.
#[derive(Debug, Default)]
pub struct Throttle {
    configs: HashMap<String, BucketConfig>,
    buckets: HashMap<String, Bucket>,
    default: BucketConfig,
}

impl Throttle {
    pub fn new(default: BucketConfig) -> Self {
        Self {
            configs: HashMap::new(),
            buckets: HashMap::new(),
            default,
        }
    }

    pub fn set_endpoint(&mut self, operation: &str, config: BucketConfig) {
        self.configs.insert(operation.to_string(), config);
        // Reset live state so the new config takes effect immediately.
        self.buckets.remove(operation);
    }

    fn config_for(&self, operation: &str) -> BucketConfig {
        self.configs.get(operation).copied().unwrap_or(self.default)
    }

    /// Seconds the caller must wait before firing for `operation` at `now_secs`
    /// (0 = fire immediately, consuming one token).
    pub fn wait_secs(&mut self, operation: &str, now_secs: f64) -> f64 {
        let config = self.config_for(operation);
        let bucket = self.buckets.entry(operation.to_string()).or_insert(Bucket {
            config,
            tokens: config.burst,
            last_secs: now_secs,
        });
        // Config may have changed since the bucket was created.
        bucket.config = config;
        let elapsed = (now_secs - bucket.last_secs).max(0.0);
        bucket.tokens = (bucket.tokens + elapsed * bucket.config.rps).min(bucket.config.burst);
        bucket.last_secs = now_secs;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            0.0
        } else {
            (1.0 - bucket.tokens) / bucket.config.rps
        }
    }
}

/// Jittered inter-request delay: `base * uniform(0.7, 1.5)` (mirrors
/// `_fetch_timeline`'s sleep). `u01` is a caller-supplied uniform in [0,1) so
/// tests can pin it.
pub fn jittered_delay_secs(base_secs: f64, u01: f64) -> f64 {
    base_secs * (0.7 + 0.8 * u01.clamp(0.0, 1.0))
}

/// Per-page count: `min(remaining + 5, 40)` (mirrors `_fetch_timeline`).
pub fn page_count(remaining: usize) -> usize {
    (remaining + 5).min(40)
}

/// POST-required ops per the Python original (search/followers/following and
/// other POST-migrated endpoints use POST; the rest use GET).
pub const POST_OPS: &[&str] = &["SearchTimeline", "Followers", "Following"];

pub fn use_post(operation: &str) -> bool {
    POST_OPS.contains(&operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_allows_immediate_then_throttles() {
        let mut t = Throttle::new(BucketConfig {
            rps: 1.0,
            burst: 2.0,
        });
        assert_eq!(t.wait_secs("Op", 0.0), 0.0);
        assert_eq!(t.wait_secs("Op", 0.0), 0.0);
        // Third immediate call must wait ~1s at 1 rps.
        let wait = t.wait_secs("Op", 0.0);
        assert!((wait - 1.0).abs() < 1e-9, "{wait}");
        // After 1s, a token refilled.
        assert_eq!(t.wait_secs("Op", 1.0), 0.0);
    }

    #[test]
    fn per_endpoint_config_overrides_default() {
        let mut t = Throttle::new(BucketConfig {
            rps: 100.0,
            burst: 100.0,
        });
        t.set_endpoint(
            "Slow",
            BucketConfig {
                rps: 0.5,
                burst: 1.0,
            },
        );
        assert_eq!(t.wait_secs("Slow", 0.0), 0.0);
        assert!(t.wait_secs("Slow", 0.0) > 1.0);
        assert_eq!(t.wait_secs("Fast", 0.0), 0.0);
    }

    #[test]
    fn page_count_matches_python_formula() {
        assert_eq!(page_count(20), 25);
        assert_eq!(page_count(35), 40);
        assert_eq!(page_count(100), 40);
        assert_eq!(page_count(1), 6);
    }

    #[test]
    fn jitter_stays_in_0_7x_to_1_5x_band() {
        assert_eq!(jittered_delay_secs(2.0, 0.0), 1.4);
        assert_eq!(jittered_delay_secs(2.0, 1.0), 3.0);
    }

    #[test]
    fn post_ops_match_python() {
        assert!(use_post("SearchTimeline"));
        assert!(use_post("Followers"));
        assert!(use_post("Following"));
        assert!(!use_post("UserByScreenName"));
        assert!(!use_post("TweetDetail"));
    }
}
