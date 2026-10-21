//! ID/URL normalization + `show N` last-list cache (plan §6).
//!
//! `tweet`/`article` accept a raw numeric ID or a full x.com URL (normalized
//! to the numeric ID). `show N` indexes into `~/.twr/last.json`, written
//! after every list command — out-of-range is exit 3 (not-found, the index
//! doesn't resolve to anything), never exit 1.

/// Extract the numeric tweet ID from a raw ID or an x.com status URL.
/// Accepts `123`, `https://x.com/u/status/123`, `.../status/123/photo/1`,
/// trailing slashes and query strings.
pub fn normalize_tweet_id(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }
    let marker = "/status/";
    let pos = s.find(marker)?;
    let rest = s.split_at(pos + marker.len()).1;
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let id = &rest[..end];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// Extract a list ID (numeric) from a raw ID or URL. Same shape as tweets
/// (`/lists/<id>`); falls back to the raw trimmed input when it looks like
/// a bare ID.
pub fn normalize_list_id(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_string());
    }
    let marker = "/lists/";
    let pos = s.find(marker)?;
    let rest = s.split_at(pos + marker.len()).1;
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let id = &rest[..end];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

/// A bare numeric X user id, if the argument is one.
///
/// X user ids are decimal digits; anything else is a screen name that still
/// has to be resolved (`UserByScreenName`) before it can be used as a
/// `userId`. Sending a screen name as `userId` used to come back as an
/// empty list, which the caller could not tell apart from "this account
/// follows nobody" (issue #2).
pub fn numeric_user_id(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(s.to_string())
}

/// Whether a string can be an X screen name: `[A-Za-z0-9_]{1,15}`.
///
/// A user argument outside that shape is neither an id nor a handle, so it
/// is a usage error rather than a lookup that would come back empty.
pub fn is_screen_name(input: &str) -> bool {
    !input.is_empty()
        && input.len() <= 15
        && input.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Default last-list path: `~/.twr/last.json`.
pub fn default_last_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| std::path::PathBuf::from(h).join(".twr").join("last.json"))
}

/// Write the last list result (tweet IDs in order) for `show N`.
pub fn write_last(path: &std::path::Path, ids: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string(&serde_json::json!({"ids": ids}))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, raw)
}

/// Read the Nth (0-based) ID from the last-list cache. `None` = missing
/// file, bad JSON, or out of range — all map to exit 3, never exit 1.
pub fn read_last_nth(path: &std::path::Path, n: usize) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("ids")?
        .as_array()?
        .get(n)?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tweet_id_from_bare_and_urls() {
        assert_eq!(normalize_tweet_id("123"), Some("123".into()));
        assert_eq!(
            normalize_tweet_id("https://x.com/u/status/123"),
            Some("123".into())
        );
        assert_eq!(
            normalize_tweet_id("https://twitter.com/u/status/456/photo/1?x=1"),
            Some("456".into())
        );
        assert_eq!(normalize_tweet_id("https://x.com/u/status/"), None);
        assert_eq!(normalize_tweet_id("not a url"), None);
        assert_eq!(normalize_tweet_id(""), None);
    }

    #[test]
    fn user_id_and_screen_name_shapes_are_told_apart() {
        assert_eq!(numeric_user_id("123"), Some("123".into()));
        assert_eq!(numeric_user_id("  123  "), Some("123".into()));
        assert_eq!(numeric_user_id("nasa"), None);
        assert_eq!(numeric_user_id(""), None);
        assert_eq!(numeric_user_id("12a"), None);

        assert!(is_screen_name("nasa"));
        assert!(is_screen_name("a_b_c"));
        assert!(is_screen_name("User123"));
        assert!(!is_screen_name(""));
        assert!(!is_screen_name("has space"));
        assert!(!is_screen_name("has-a-dash"));
        assert!(!is_screen_name("sixteencharacters"));
    }

    #[test]
    fn last_json_round_trip_and_out_of_range_is_none() {
        let dir = std::env::temp_dir().join(format!("twr-last-{}", std::process::id()));
        let path = dir.join("last.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_last_nth(&path, 0).is_none());
        write_last(&path, &["a".into(), "b".into()]).unwrap();
        assert_eq!(read_last_nth(&path, 1), Some("b".into()));
        assert!(read_last_nth(&path, 5).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
