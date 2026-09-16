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

/// DM daily cap (bead o1l.5.3): mirrors the general budget's exact
/// pattern — own state file, own env override, same clamp-never-disable
/// semantics, SAME exit code 2 / BudgetCheck::Deny shape (a LOCAL POLICY
/// denial, not a server rate-limit — never exit 4 / retryAfterMs).
/// Default 10: DMs are the highest-scrutiny surface (see SKILL.md §6);
/// 10/day is enough for solicited replies, far below any spam profile.
pub const DEFAULT_DM_DAILY_BUDGET: u32 = 10;
/// Env override name for the DM cap.
pub const DM_BUDGET_ENV_VAR: &str = "TWR_DM_DAILY_BUDGET";

/// Effective DM budget: env override clamped to `[MIN, u32::MAX]`, else default.
pub fn effective_dm_budget(read_env: impl FnOnce(&str) -> Option<String>) -> u32 {
    match read_env(DM_BUDGET_ENV_VAR).and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => n.max(MIN_DAILY_BUDGET),
        None => DEFAULT_DM_DAILY_BUDGET,
    }
}

/// Default DM state path: `~/.twr/dm_budget.json` (sibling to mutations.json).
pub fn default_dm_log_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".twr").join("dm_budget.json"))
}

/// DM denial suggestion (exit 2).
pub fn dm_denial_suggestion(used: u32, limit: u32) -> String {
    format!("daily DM budget exhausted ({used}/{limit} today); raise with TWR_DM_DAILY_BUDGET (never disableable) or wait until tomorrow — DMs are the highest-scrutiny surface, see SKILL.md §6")
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
    fn dm_budget_defaults_and_env_override() {
        assert_eq!(effective_dm_budget(|_| None), 10);
        assert_eq!(effective_dm_budget(|_| Some("3".into())), 3);
        assert_eq!(effective_dm_budget(|_| Some("0".into())), 1);
        assert_eq!(effective_dm_budget(|_| Some("junk".into())), 10);
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

    /// Bead o1l.5.4 §3 — DM-cap vs general-budget distinguishability: an
    /// agent that only knows the general-budget error MUST still recognize
    /// the DM-cap error as a different condition. The two denial strings
    /// share nothing except the tail advice clause: different resource
    /// noun ("DM budget" vs "mutation budget"), different env var
    /// (TWR_DM_DAILY_BUDGET vs TWR_DAILY_BUDGET). Same exit-2 Deny shape
    /// (both are local-policy denials, never exit 4). Separate files:
    /// recording DM usage must not touch the general counter and back.
    #[test]
    fn p54_dm_cap_denial_is_distinguishable_from_general_budget() {
        let dm = dm_denial_suggestion(10, 10);
        let general = denial_suggestion(200, 200);
        assert!(dm.contains("DM budget"), "DM denial names itself: {dm}");
        assert!(
            dm.contains("TWR_DM_DAILY_BUDGET"),
            "DM denial names its var"
        );
        assert!(
            general.contains("mutation budget"),
            "general denial names itself"
        );
        assert!(
            general.contains("TWR_DAILY_BUDGET"),
            "general denial names its var"
        );
        assert!(!dm.contains("mutation budget"), "no noun bleed: {dm}");
        assert!(!general.contains("DM budget"), "no noun bleed: {general}");
        // Counters are independent: exhausting one leaves the other Allow.
        let dir = std::env::temp_dir().join(format!("twr-dmcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dm_path = dir.join("dm_budget.json");
        let gen_path = dir.join("mutations.json");
        record(&dm_path, "2026-09-16");
        assert_eq!(
            check(&dm_path, "2026-09-16", 1),
            BudgetCheck::Deny { used: 1, limit: 1 }
        );
        assert_eq!(
            check(&gen_path, "2026-09-16", 1),
            BudgetCheck::Allow { used: 0, limit: 1 },
            "DM spend must not consume the general budget"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
