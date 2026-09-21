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

/// Post daily cap + min-interval cooldown (2026-09-20 incident): X's
/// code-226 automated-behavior gate fires on CreateTweet-family ops long
/// before the general 200/day budget matters — 21 mixed mutations in ~1h
/// plus rapid post/delete/retry loops got the account flagged while the
/// general counter still showed headroom. So posts (post/reply/quote/edit)
/// get their OWN daily counter AND a minimum gap between two posts.
/// Same exit-2 Deny shape as the DM cap (local-policy denial, never exit 4).
/// Default 10/day + 15min gap: enough for real human-paced use, far below
/// any automation profile.
pub const DEFAULT_POST_DAILY_BUDGET: u32 = 10;
/// Env override name for the post cap.
pub const POST_BUDGET_ENV_VAR: &str = "TWR_POST_DAILY_BUDGET";
/// Default minimum seconds between two post-family writes.
pub const DEFAULT_POST_MIN_INTERVAL_SECS: u64 = 900;
/// Env override name for the post cooldown.
pub const POST_INTERVAL_ENV_VAR: &str = "TWR_POST_MIN_INTERVAL_SECS";

/// Effective post budget: env override clamped to `[MIN, u32::MAX]`, else default.
pub fn effective_post_budget(read_env: impl FnOnce(&str) -> Option<String>) -> u32 {
    match read_env(POST_BUDGET_ENV_VAR).and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => n.max(MIN_DAILY_BUDGET),
        None => DEFAULT_POST_DAILY_BUDGET,
    }
}

/// Effective post cooldown: env override in seconds, floor 0 (=disabled),
/// else default 900s. A floor of 0 (not 1) is deliberate: the cooldown is
/// a pacing aid, not a safety invariant — the daily cap above is the
/// never-disableable gate.
pub fn effective_post_min_interval(read_env: impl FnOnce(&str) -> Option<String>) -> u64 {
    match read_env(POST_INTERVAL_ENV_VAR).and_then(|v| v.parse::<u64>().ok()) {
        Some(n) => n,
        None => DEFAULT_POST_MIN_INTERVAL_SECS,
    }
}

/// Default post state path: `~/.twr/post_budget.json` (sibling to
/// mutations.json / dm_budget.json).
pub fn default_post_log_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".twr").join("post_budget.json"))
}

/// Post-budget state: per-day counters (pruned to today on write) plus the
/// timestamp of the last successful post (for the cooldown).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PostLog {
    #[serde(default)]
    pub days: MutationLog,
    #[serde(default)]
    pub last_post_secs: u64,
}

fn load_post_log(path: &Path) -> PostLog {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_post_log(path: &Path, log: &PostLog) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string(log) {
        let _ = std::fs::write(path, raw);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostCheck {
    /// Under cap and past the cooldown; caller records after success.
    Allow { used: u32, limit: u32 },
    /// At/over daily cap — exit 2.
    DenyBudget { used: u32, limit: u32 },
    /// Daily cap fine but last post was too recent — exit 2 with wait hint.
    DenyCooldown { wait_secs: u64 },
}

/// Check the post cap AND the cooldown. `now_secs` is epoch seconds.
pub fn check_post(
    path: &Path,
    today: &str,
    limit: u32,
    min_interval_secs: u64,
    now_secs: u64,
) -> PostCheck {
    let log = load_post_log(path);
    let used = log.days.get(today).copied().unwrap_or(0);
    if used >= limit {
        return PostCheck::DenyBudget { used, limit };
    }
    if min_interval_secs > 0 && log.last_post_secs > 0 {
        if let Some(elapsed) = now_secs.checked_sub(log.last_post_secs) {
            if elapsed < min_interval_secs {
                return PostCheck::DenyCooldown {
                    wait_secs: min_interval_secs - elapsed,
                };
            }
        }
    }
    PostCheck::Allow { used, limit }
}

/// Record one successful post-family write for today (prunes stale days).
pub fn record_post(path: &Path, today: &str, now_secs: u64) {
    let mut log = load_post_log(path);
    log.days.retain(|k, _| k == today);
    *log.days.entry(today.to_string()).or_insert(0) += 1;
    log.last_post_secs = now_secs;
    save_post_log(path, &log);
}

/// Post-cap denial suggestion (exit 2).
pub fn post_denial_suggestion(used: u32, limit: u32) -> String {
    format!("daily post budget exhausted ({used}/{limit} today); raise with TWR_POST_DAILY_BUDGET (never disableable) or wait until tomorrow — CreateTweet-family ops are X's most-scrutinized write surface after DMs")
}

/// Cooldown denial suggestion (exit 2, includes the wait).
pub fn post_cooldown_suggestion(wait_secs: u64) -> String {
    format!("post cooldown active — wait ~{wait_secs}s before the next post (TWR_POST_MIN_INTERVAL_SECS, default 900s). Back-to-back posts are exactly the automation pattern X's code-226 gate flags")
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

    /// Post cap + cooldown: independent counters, distinguishable denials.
    #[test]
    fn post_cap_and_cooldown() {
        assert_eq!(effective_post_budget(|_| None), 10);
        assert_eq!(effective_post_budget(|_| Some("3".into())), 3);
        assert_eq!(effective_post_budget(|_| Some("0".into())), 1);
        assert_eq!(effective_post_min_interval(|_| None), 900);
        assert_eq!(effective_post_min_interval(|_| Some("60".into())), 60);
        let dir = std::env::temp_dir().join(format!("twr-postcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let post_path = dir.join("post_budget.json");
        let gen_path = dir.join("mutations.json");
        // Fresh: allow.
        assert_eq!(
            check_post(&post_path, "2026-09-20", 2, 900, 10000),
            PostCheck::Allow { used: 0, limit: 2 }
        );
        // Record one post at t=10000; immediate retry hits cooldown.
        record_post(&post_path, "2026-09-20", 10000);
        assert_eq!(
            check_post(&post_path, "2026-09-20", 2, 900, 10300),
            PostCheck::DenyCooldown { wait_secs: 600 }
        );
        // Past the cooldown: allow again with used=1.
        assert_eq!(
            check_post(&post_path, "2026-09-20", 2, 900, 10900),
            PostCheck::Allow { used: 1, limit: 2 }
        );
        record_post(&post_path, "2026-09-20", 10900);
        // Cap exhausted (cooldown irrelevant now).
        assert_eq!(
            check_post(&post_path, "2026-09-20", 2, 900, 20000),
            PostCheck::DenyBudget { used: 2, limit: 2 }
        );
        // General budget untouched by post spend.
        assert_eq!(
            check(&gen_path, "2026-09-20", 1),
            BudgetCheck::Allow { used: 0, limit: 1 },
            "post spend must not consume the general budget"
        );
        // Denials name their own resource/var.
        let d = post_denial_suggestion(10, 10);
        assert!(
            d.contains("post budget") && d.contains("TWR_POST_DAILY_BUDGET"),
            "{d}"
        );
        let c = post_cooldown_suggestion(600);
        assert!(c.contains("600") && c.contains("226"), "{c}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
