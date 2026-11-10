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
/// Hard ceiling for every daily-count env override. The budget can be
/// raised but never disabled — and never set to an absurd value either
/// (a typo like `TWR_DAILY_BUDGET=999999999` must not silently uncap the
/// safety policy).
pub const MAX_DAILY_BUDGET: u32 = 10_000;
/// Env override for the state directory (bead nku): points at the `.twr`
/// state dir itself (the directory that holds `mutations.json`,
/// `dm_budget.json`, `post_budget.json`, `idempotency.json`,
/// `query-ids.json`). Lets sandboxed/container environments without HOME
/// keep fail-closed budgets.
pub const STATE_DIR_ENV_VAR: &str = "TWR_STATE_DIR";

/// Effective budget: env override clamped to `[MIN, MAX]`, else default.
pub fn effective_budget(read_env: impl FnOnce(&str) -> Option<String>) -> u32 {
    match read_env(BUDGET_ENV_VAR).and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => n.clamp(MIN_DAILY_BUDGET, MAX_DAILY_BUDGET),
        None => DEFAULT_DAILY_BUDGET,
    }
}

pub type MutationLog = HashMap<String, u32>;

/// The `.twr` state directory: `TWR_STATE_DIR` when set, else
/// `$HOME/.twr` / `%USERPROFILE%\.twr`. `None` means NO writable state
/// location exists — write paths MUST fail closed on this (a missing
/// budget file must never silently mean "unlimited mutations").
pub fn state_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(STATE_DIR_ENV_VAR) {
        let dir = PathBuf::from(dir);
        if !dir.as_os_str().is_empty() {
            return Some(dir);
        }
    }
    home_dir().map(|h| h.join(".twr"))
}

/// Fail-closed variant for write paths: the state dir or a clear error.
pub fn require_state_dir() -> Result<PathBuf, String> {
    state_dir().ok_or_else(|| {
        format!("no writable state dir (HOME/USERPROFILE and {STATE_DIR_ENV_VAR} are all unset) — refusing a write whose safety budget cannot be enforced; set {STATE_DIR_ENV_VAR} to a writable directory")
    })
}

/// Default state path: `<state_dir>/mutations.json`.
pub fn default_log_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("mutations.json"))
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

/// Write `raw` to `path` atomically: a sibling temp file then rename, so a
/// crash mid-write can never leave a truncated counter (which would read
/// back as 0 and silently reopen the budget).
pub fn atomic_write(path: &Path, raw: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&tmp, raw)?;
    // Windows rename refuses to replace an existing file; removing first
    // reopens a tiny gap there, but unix rename(2) is atomic-replace and
    // this is the best std-only portable shape.
    #[cfg(windows)]
    let _ = std::fs::remove_file(path);
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Cross-process mutex for state read-modify-write cycles. `twr` is a
/// one-shot CLI: two concurrent invocations can otherwise both pass
/// `check` at used=limit-1 and both `record`, exceeding the cap. The lock
/// is a `create_new` lockfile beside the state file; a holder that dies
/// leaves it behind, so entries older than [`LOCK_STALE_SECS`] are broken.
const LOCK_STALE_SECS: u64 = 30;
const LOCK_ATTEMPTS: u32 = 200;
const LOCK_RETRY_MS: u64 = 25;

pub struct StateLock {
    path: PathBuf,
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn lock_path_for(path: &Path) -> PathBuf {
    path.with_extension("lock")
}

/// Acquire the per-state-file lock. `Err` on timeout — callers of safety
/// state MUST treat that as fail-closed, never proceed unlocked.
pub fn acquire_state_lock(path: &Path) -> std::io::Result<StateLock> {
    let lock = lock_path_for(path);
    // The state dir may not exist yet on first use — create it like
    // `atomic_write` does, or the lockfile open fails ENOENT and every
    // guarded write silently no-ops (fail-open).
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for attempt in 0..LOCK_ATTEMPTS {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(_) => return Ok(StateLock { path: lock }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Stale-lock break: a crashed holder leaves the file behind.
                let stale = std::fs::metadata(&lock)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() >= LOCK_STALE_SECS);
                if stale {
                    let _ = std::fs::remove_file(&lock);
                    continue;
                }
                if attempt + 1 == LOCK_ATTEMPTS {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(LOCK_RETRY_MS));
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::WouldBlock,
        format!("timed out acquiring state lock {}", lock.display()),
    ))
}

/// Run `f` while holding the state lock for `path`.
pub fn with_state_lock<T>(path: &Path, f: impl FnOnce() -> T) -> std::io::Result<T> {
    let _guard = acquire_state_lock(path)?;
    Ok(f())
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

/// Record one successful mutation for today. Atomic (tmp+rename) and
/// locked against concurrent writers.
pub fn record(path: &Path, today: &str) {
    let _ = with_state_lock(path, || {
        let mut log: MutationLog = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        // Prune stale days: keep only today.
        log.retain(|k, _| k == today);
        *log.entry(today.to_string()).or_insert(0) += 1;
        if let Ok(raw) = serde_json::to_string(&log) {
            let _ = atomic_write(path, &raw);
        }
    });
}

/// Atomically check-and-RESERVE one mutation (bead nku): under the state
/// lock, read today's count, deny at the cap, else increment and persist
/// BEFORE the caller sends. This closes the check-then-record race two
/// concurrent `twr` invocations had: both could pass `check` and both
/// record, exceeding the cap.
///
/// A reservation counts an ATTEMPT that reached the send, not only a
/// confirmed success — the unsafe direction is under-counting, so failed
/// sends still consume budget (documented, deliberate).
///
/// `Err` = state failure (lock timeout / unwritable) — callers MUST fail
/// closed (exit 2), never proceed unbudgeted.
pub fn reserve(path: &Path, today: &str, limit: u32) -> Result<BudgetCheck, String> {
    with_state_lock(path, || {
        let mut log: MutationLog = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        log.retain(|k, _| k == today);
        let used = log.get(today).copied().unwrap_or(0);
        if used >= limit {
            return Ok(BudgetCheck::Deny { used, limit });
        }
        *log.entry(today.to_string()).or_insert(0) += 1;
        let raw = serde_json::to_string(&log)
            .map_err(|e| format!("cannot serialize mutation budget: {e}"))?;
        atomic_write(path, &raw).map_err(|e| format!("cannot write mutation budget: {e}"))?;
        Ok(BudgetCheck::Allow {
            used: used + 1,
            limit,
        })
    })
    .map_err(|e| format!("cannot lock mutation budget state: {e}"))?
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

/// Effective DM budget: env override clamped to `[MIN, MAX]`, else default.
pub fn effective_dm_budget(read_env: impl FnOnce(&str) -> Option<String>) -> u32 {
    match read_env(DM_BUDGET_ENV_VAR).and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => n.clamp(MIN_DAILY_BUDGET, MAX_DAILY_BUDGET),
        None => DEFAULT_DM_DAILY_BUDGET,
    }
}

/// Default DM state path: `<state_dir>/dm_budget.json`.
pub fn default_dm_log_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("dm_budget.json"))
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

/// Effective post budget: env override clamped to `[MIN, MAX]`, else default.
pub fn effective_post_budget(read_env: impl FnOnce(&str) -> Option<String>) -> u32 {
    match read_env(POST_BUDGET_ENV_VAR).and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => n.clamp(MIN_DAILY_BUDGET, MAX_DAILY_BUDGET),
        None => DEFAULT_POST_DAILY_BUDGET,
    }
}

/// Floor for the post cooldown (bead nku): `TWR_POST_MIN_INTERVAL_SECS=0`
/// must NOT disable pacing — the daily cap alone let burst posts through
/// between midnight resets. 60s is the smallest honest gap; the ceiling
/// stays 24h so an unbounded value can't freeze posting for weeks.
pub const MIN_POST_MIN_INTERVAL_SECS: u64 = 60;
/// Hard ceiling for the post cooldown (24h).
pub const MAX_POST_MIN_INTERVAL_SECS: u64 = 24 * 3600;

/// Effective post cooldown: env override in seconds, clamped to
/// `[MIN_POST_MIN_INTERVAL_SECS, MAX_POST_MIN_INTERVAL_SECS]`, else
/// default 900s. Zero is NOT honored — see the floor constant.
pub fn effective_post_min_interval(read_env: impl FnOnce(&str) -> Option<String>) -> u64 {
    match read_env(POST_INTERVAL_ENV_VAR).and_then(|v| v.parse::<u64>().ok()) {
        Some(n) => n.clamp(MIN_POST_MIN_INTERVAL_SECS, MAX_POST_MIN_INTERVAL_SECS),
        None => DEFAULT_POST_MIN_INTERVAL_SECS,
    }
}

/// Default post state path: `<state_dir>/post_budget.json`.
pub fn default_post_log_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("post_budget.json"))
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
    if let Ok(raw) = serde_json::to_string(log) {
        let _ = atomic_write(path, &raw);
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
/// Atomic (tmp+rename) and locked against concurrent writers.
pub fn record_post(path: &Path, today: &str, now_secs: u64) {
    let _ = with_state_lock(path, || {
        let mut log = load_post_log(path);
        log.days.retain(|k, _| k == today);
        *log.days.entry(today.to_string()).or_insert(0) += 1;
        log.last_post_secs = now_secs;
        save_post_log(path, &log);
    });
}

/// Atomically check-and-RESERVE one post-family write (bead nku): same
/// contract as [`reserve`] — under the state lock, deny at the daily cap
/// or inside the cooldown, else increment the day counter, stamp
/// `last_post_secs`, and persist BEFORE the caller sends. A reservation
/// counts an ATTEMPT (a failed send still consumes cap + starts the
/// cooldown — the unsafe direction is under-counting).
///
/// `Err` = state failure — callers MUST fail closed (exit 2).
pub fn reserve_post(
    path: &Path,
    today: &str,
    limit: u32,
    min_interval_secs: u64,
    now_secs: u64,
) -> Result<PostCheck, String> {
    with_state_lock(path, || {
        let mut log = load_post_log(path);
        let used = log.days.get(today).copied().unwrap_or(0);
        if used >= limit {
            return Ok(PostCheck::DenyBudget { used, limit });
        }
        if min_interval_secs > 0 && log.last_post_secs > 0 {
            if let Some(elapsed) = now_secs.checked_sub(log.last_post_secs) {
                if elapsed < min_interval_secs {
                    return Ok(PostCheck::DenyCooldown {
                        wait_secs: min_interval_secs - elapsed,
                    });
                }
            }
        }
        log.days.retain(|k, _| k == today);
        *log.days.entry(today.to_string()).or_insert(0) += 1;
        log.last_post_secs = now_secs;
        save_post_log(path, &log);
        Ok(PostCheck::Allow {
            used: used + 1,
            limit,
        })
    })
    .map_err(|e| format!("cannot lock post budget state: {e}"))?
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
