//! Daily mutation budget (bead twitter_cli-5o3.4.7).
//!
//! Explicit LOCAL SAFETY POLICY (default 200/day, x-cli-vibe pattern) —
//! independent of X's actual server-side rate limit, making no claim about
//! it. Exceeding it exits 2 with a suggestion. Configurable upward via
//! `TWR_DAILY_BUDGET`, never fully disableable (floor of 1).
//!
//! State: `~/.twr/mutations.json` mapping `YYYY-MM-DD` → count. Only today's
//! entry matters; stale days prune on load.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Default daily budget.
pub const DEFAULT_DAILY_BUDGET: u32 = 200;
/// Hard floor — the budget can be raised but never disabled.
pub const MIN_DAILY_BUDGET: u32 = 1;
/// Env override name.
pub const BUDGET_ENV_VAR: &str = "TWR_DAILY_BUDGET";

/// Effective budget: env override clamped to `[MIN, u32::MAX]`, else default.
pub fn effective_budget(read_env: impl FnOnce(&str) -> Option<String>) -> u32 {
    match read_env(BUDGET_ENV_VAR).and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => n.max(MIN_DAILY_BUDGET),
        None => DEFAULT_DAILY_BUDGET,
    }
}

pub type MutationLog = HashMap<String, u32>;

/// Default state path: `~/.twr/mutations.json`.
pub fn default_log_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".twr").join("mutations.json"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Today as `YYYY-MM-DD` (UTC). Split for testability.
pub fn today_utc() -> String {
    // Days since epoch → civil date (Howard Hinnant's algorithm).
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86400)
        .unwrap_or(0) as i64;
    days_to_civil(days)
}

fn days_to_civil(z: i64) -> String {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetCheck {
    /// Under budget; caller should record after success.
    Allow { used: u32, limit: u32 },
    /// At/over budget — exit 2 with suggestion.
    Deny { used: u32, limit: u32 },
}

/// Load today's count (missing/garbage = 0) and check against the budget.
pub fn check(path: &Path, today: &str, limit: u32) -> BudgetCheck {
    let used = load_count(path, today);
    if used >= limit {
        BudgetCheck::Deny { used, limit }
    } else {
        BudgetCheck::Allow { used, limit }
    }
}

fn load_count(path: &Path, today: &str) -> u32 {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return 0;
    };
    serde_json::from_str::<MutationLog>(&raw)
        .ok()
        .and_then(|m| m.get(today).copied())
        .unwrap_or(0)
}

/// Record one successful mutation for today.
pub fn record(path: &Path, today: &str) {
    let mut log: MutationLog = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    // Prune stale days: keep only today.
    log.retain(|k, _| k == today);
    *log.entry(today.to_string()).or_insert(0) += 1;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string(&log) {
        let _ = std::fs::write(path, raw);
    }
}

/// Denial suggestion (exit 2).
pub fn denial_suggestion(used: u32, limit: u32) -> String {
    format!("daily mutation budget exhausted ({used}/{limit} today); raise with TWR_DAILY_BUDGET (never disableable) or wait until tomorrow")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_defaults_and_env_override() {
        assert_eq!(effective_budget(|_| None), 200);
        assert_eq!(effective_budget(|_| Some("50".into())), 50);
        assert_eq!(effective_budget(|_| Some("0".into())), 1);
        assert_eq!(effective_budget(|_| Some("junk".into())), 200);
    }

    #[test]
    fn today_formats_as_date() {
        let t = today_utc();
        assert_eq!(t.len(), 10);
        assert_eq!(&t[4..5], "-");
    }

    #[test]
    fn check_and_record_round_trip() {
        let dir = std::env::temp_dir().join(format!("twr-budget-{}", std::process::id()));
        let path = dir.join("mutations.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            check(&path, "2026-09-15", 2),
            BudgetCheck::Allow { used: 0, limit: 2 }
        );
        record(&path, "2026-09-15");
        record(&path, "2026-09-15");
        assert_eq!(
            check(&path, "2026-09-15", 2),
            BudgetCheck::Deny { used: 2, limit: 2 }
        );
        // Other day unaffected.
        assert_eq!(
            check(&path, "2026-09-16", 2),
            BudgetCheck::Allow { used: 0, limit: 2 }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
