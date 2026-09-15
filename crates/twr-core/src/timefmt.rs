//! Time display: --time relative|absolute|both (issue #35).
//!
//! The Python original only shows relative time ("28s ago"). twr adds a
//! global --time flag (default relative, preserving parity); absolute mode
//! is ISO 8601 with local offset.
//!
//! CRITICAL: machine output (json/yaml/toon) ALWAYS carries the absolute
//! `created_at` regardless of this flag — only the human table's display is
//! affected. Agents must never parse relative-time strings.
//!
//! Dependency-free (no chrono): Twitter timestamps
//! ("Sat Mar 08 12:00:00 +0000 2026") are parsed by hand.

/// Display mode for the human table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeMode {
    #[default]
    Relative,
    Absolute,
    Both,
}

impl TimeMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "relative" => Some(TimeMode::Relative),
            "absolute" => Some(TimeMode::Absolute),
            "both" => Some(TimeMode::Both),
            _ => None,
        }
    }
}

fn month_num(m: &str) -> Option<u32> {
    match m {
        "Jan" => Some(1),
        "Feb" => Some(2),
        "Mar" => Some(3),
        "Apr" => Some(4),
        "May" => Some(5),
        "Jun" => Some(6),
        "Jul" => Some(7),
        "Aug" => Some(8),
        "Sep" => Some(9),
        "Oct" => Some(10),
        "Nov" => Some(11),
        "Dec" => Some(12),
        _ => None,
    }
}

/// Parsed Twitter timestamp → unix seconds (UTC). None on parse failure.
pub fn parse_twitter_time(created_at: &str) -> Option<i64> {
    // "Sat Mar 08 12:00:00 +0000 2026"
    let parts: Vec<&str> = created_at.split_whitespace().collect();
    if parts.len() != 6 {
        return None;
    }
    let month = month_num(parts[1])?;
    let day: u32 = parts[2].parse().ok()?;
    let t: Vec<&str> = parts[3].split(':').collect();
    if t.len() != 3 {
        return None;
    }
    let (hh, mm, ss): (u32, u32, u32) =
        (t[0].parse().ok()?, t[1].parse().ok()?, t[2].parse().ok()?);
    let tz = parts[4];
    let year: i32 = parts[5].parse().ok()?;
    // Offset minutes from ±HHMM.
    let sign = match tz.chars().next()? {
        '+' => 1i64,
        '-' => -1i64,
        _ => return None,
    };
    let off_h: i64 = tz.get(1..3)?.parse().ok()?;
    let off_m: i64 = tz.get(3..5)?.parse().ok()?;
    let days = days_from_civil(year, month, day)?;
    let local = days * 86400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64;
    Some(local - sign * (off_h * 3600 + off_m * 60))
}

fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146097 + doe - 719468)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Relative display ("28s/5m/3h/9d ago"), mirroring Python's buckets.
pub fn relative(created_at: &str, now: i64) -> String {
    let Some(ts) = parse_twitter_time(created_at) else {
        return created_at.to_string();
    };
    let delta = (now - ts).max(0);
    if delta < 60 {
        format!("{delta}s ago")
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86400 {
        format!("{}h ago", delta / 3600)
    } else {
        format!("{}d ago", delta / 86400)
    }
}

/// Absolute display: ISO 8601 with local offset. Without a tz database we
/// render UTC (`+00:00`) — unambiguous for agents and humans alike; the
/// contract only requires absolute, not wall-clock-local.
pub fn absolute(created_at: &str) -> String {
    let Some(ts) = parse_twitter_time(created_at) else {
        return created_at.to_string();
    };
    civil_iso(ts)
}

fn civil_iso(ts: i64) -> String {
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 {
        (mp + 3) as u32
    } else {
        (mp - 9) as u32
    };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Display per --time mode (human table only).
pub fn display(created_at: &str, mode: TimeMode) -> String {
    match mode {
        TimeMode::Relative => relative(created_at, now_secs()),
        TimeMode::Absolute => absolute(created_at),
        TimeMode::Both => format!(
            "{} ({})",
            relative(created_at, now_secs()),
            absolute(created_at)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Sat Mar 08 12:00:00 +0000 2026";

    #[test]
    fn parses_sample() {
        // 2026-03-08T12:00:00Z.
        let ts = parse_twitter_time(SAMPLE).unwrap();
        assert_eq!(civil_iso(ts), "2026-03-08T12:00:00+00:00");
        assert!(parse_twitter_time("junk").is_none());
    }

    #[test]
    fn relative_buckets() {
        let ts = parse_twitter_time(SAMPLE).unwrap();
        assert_eq!(relative(SAMPLE, ts + 28), "28s ago");
        assert_eq!(relative(SAMPLE, ts + 5 * 60), "5m ago");
        assert_eq!(relative(SAMPLE, ts + 3 * 3600), "3h ago");
        assert_eq!(relative(SAMPLE, ts + 9 * 86400), "9d ago");
        assert_eq!(relative("junk", ts), "junk");
    }

    #[test]
    fn absolute_is_iso_and_mode_parses() {
        assert_eq!(absolute(SAMPLE), "2026-03-08T12:00:00+00:00");
        assert_eq!(TimeMode::parse("both"), Some(TimeMode::Both));
        assert_eq!(TimeMode::parse("nope"), None);
    }
}
