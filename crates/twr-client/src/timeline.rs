//! `_fetch_timeline` pagination loop, ported from `client.py` (plan §7).
//!
//! Rules carried over verbatim:
//! - `count = min(count, max_count)`; `count <= 0` → empty.
//! - `max_attempts = ceil(count / 20) + 2`.
//! - Per page: `count = min(remaining + 5, 40)`.
//! - Id dedup across pages (`seen_ids`); truncate to `count` at the end.
//! - Stop when the cursor disappears or stops advancing; `return_cursor`
//!   yields the continuation cursor for `--cursor` resume.
//! - Rate-limit resume: a page reporting `rate_limited` ships partial data
//!   with `truncated=true` instead of erroring (plan §5.2 exit-4 row).
//!
//! The HTTP per page is injected (`fetch_page`), so this is pure logic +
//! unit-testable. Parsing bytes → tweets stays in `twr-model`.

/// One fetched page: new IDs in order, plus the next cursor (if any).
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub ids: Vec<String>,
    pub next_cursor: Option<String>,
}

/// A page-fetch failure the loop knows how to handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageError {
    /// 429 / inner 88/348/349 — ship partial data, mark truncated.
    RateLimited,
    /// Anything else — abort the whole fetch.
    Fatal,
}

/// Outcome of [`fetch_timeline`].
#[derive(Debug, Clone, Default)]
pub struct TimelineResult {
    pub ids: Vec<String>,
    /// `Some` only when `return_cursor` was set.
    pub continuation_cursor: Option<String>,
    /// True when a mid-loop rate limit cut pagination short (exit 4 ships
    /// partial data with `meta.truncated=true`).
    pub truncated: bool,
}

/// Generic timeline fetch. `max_count` caps `count` (plan: configurable,
/// never disabled); `fetch_page(page_count, cursor)` performs one HTTP round
/// trip through the caller's transport + header builder.
pub fn fetch_timeline(
    count: usize,
    max_count: usize,
    start_cursor: Option<String>,
    return_cursor: bool,
    mut fetch_page: impl FnMut(usize, Option<&str>) -> Result<Page, PageError>,
) -> Result<TimelineResult, PageError> {
    if count == 0 {
        return Ok(TimelineResult::default());
    }
    let count = count.min(max_count);
    let max_attempts = count.div_ceil(20) + 2;

    let mut ids: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut cursor = start_cursor;
    let mut continuation_cursor = None;
    let mut truncated = false;
    let mut attempts = 0;

    while ids.len() < count && attempts < max_attempts {
        attempts += 1;
        let remaining = count - ids.len();
        let page_count = crate::throttle::page_count(remaining);
        let page = match fetch_page(page_count, cursor.as_deref()) {
            Ok(page) => page,
            Err(PageError::RateLimited) => {
                truncated = true;
                break;
            }
            Err(PageError::Fatal) => return Err(PageError::Fatal),
        };

        for id in page.ids {
            if !id.is_empty() && seen.insert(id.clone()) {
                ids.push(id);
            }
        }

        match page.next_cursor {
            None => {
                continuation_cursor = None;
                break;
            }
            Some(next) if Some(&next) == cursor.as_ref() => {
                continuation_cursor = None;
                break;
            }
            Some(next) => {
                continuation_cursor = Some(next.clone());
                cursor = Some(next);
            }
        }
    }

    ids.truncate(count);
    Ok(TimelineResult {
        ids,
        continuation_cursor: if return_cursor {
            continuation_cursor
        } else {
            None
        },
        truncated,
    })
}

/// Exponential backoff for 429: `5s base × 3 retries` per plan's
/// `rateLimit.backoff` default. Returns the delays to sleep before attempts
/// 1..=retries (attempt 0 fires immediately).
pub fn backoff_delays_secs(base_secs: f64, retries: u32) -> Vec<f64> {
    (0..retries)
        .map(|i| base_secs * 2f64.powi(i as i32))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_count_dedups_and_truncates() {
        let result = fetch_timeline(3, 200, None, false, |page_count, _| {
            assert_eq!(page_count, 8); // min(3 + 5, 40)
            Ok(Page {
                ids: vec!["a".into(), "b".into(), "a".into(), "c".into(), "d".into()],
                next_cursor: None,
            })
        })
        .unwrap();
        assert_eq!(result.ids, vec!["a", "b", "c"]);
        assert!(!result.truncated);
    }

    #[test]
    fn stops_when_cursor_stops_advancing() {
        let mut calls = 0;
        let result = fetch_timeline(50, 200, None, true, |_, cursor| {
            calls += 1;
            if calls == 1 {
                Ok(Page {
                    ids: vec!["a".into()],
                    next_cursor: Some("c1".into()),
                })
            } else {
                // Same cursor back = no advance → stop, no continuation.
                assert_eq!(cursor, Some("c1"));
                Ok(Page {
                    ids: vec![],
                    next_cursor: Some("c1".into()),
                })
            }
        })
        .unwrap();
        assert_eq!(result.ids, vec!["a"]);
        assert_eq!(result.continuation_cursor, None);
        assert_eq!(calls, 2);
    }

    #[test]
    fn rate_limit_ships_partial_with_truncated() {
        let result = fetch_timeline(50, 200, None, true, |_, _| {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                Ok(Page {
                    ids: vec!["a".into()],
                    next_cursor: Some("c1".into()),
                })
            } else {
                Err(PageError::RateLimited)
            }
        })
        .unwrap();
        assert_eq!(result.ids, vec!["a"]);
        assert!(result.truncated);
    }

    #[test]
    fn fatal_aborts() {
        let err = fetch_timeline(10, 200, None, false, |_, _| Err(PageError::Fatal)).unwrap_err();
        assert_eq!(err, PageError::Fatal);
    }

    #[test]
    fn backoff_is_5s_base_times_3() {
        assert_eq!(backoff_delays_secs(5.0, 3), vec![5.0, 10.0, 20.0]);
    }

    #[test]
    fn zero_count_short_circuits_without_fetching() {
        let result = fetch_timeline(0, 200, None, false, |_, _| panic!("must not fetch")).unwrap();
        assert!(result.ids.is_empty());
    }
}
