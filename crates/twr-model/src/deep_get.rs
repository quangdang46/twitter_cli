//! Port of `parser.py`'s `_deep_get`: safe nested dict/list navigation over
//! `serde_json::Value`, supporting both string keys and list indices in one
//! path — mirrors Python's `_deep_get(data, "a", "b", 0, "c")`.

use serde_json::Value;

#[derive(Clone, Copy)]
pub enum Seg<'a> {
    Key(&'a str),
    Idx(usize),
}

impl<'a> From<&'a str> for Seg<'a> {
    fn from(s: &'a str) -> Self {
        Seg::Key(s)
    }
}

impl From<usize> for Seg<'static> {
    fn from(i: usize) -> Self {
        Seg::Idx(i)
    }
}

pub fn deep_get<'a>(data: &'a Value, segs: &[Seg]) -> Option<&'a Value> {
    let mut cur = data;
    for seg in segs {
        cur = match seg {
            Seg::Key(k) => cur.get(*k)?,
            Seg::Idx(i) => cur.get(*i)?,
        };
    }
    Some(cur)
}

/// Builds a `&[Seg]` from mixed `&str`/`usize` path segments, exactly like
/// calling `_deep_get(data, "entities", "url", "urls", 0, "expanded_url")` in
/// Python. Usage: `dget!(&value, "a", "b", 0, "c")`.
#[macro_export]
macro_rules! dget {
    ($data:expr, $($seg:expr),+ $(,)?) => {
        $crate::deep_get::deep_get($data, &[$($crate::deep_get::Seg::from($seg)),+])
    };
}

/// Python's `_parse_int`: best-effort integer conversion that tolerates
/// commas, surrounding whitespace, and numeric-string floats (`"12.0"`).
pub fn parse_int(value: Option<&Value>, default: i64) -> i64 {
    let Some(value) = value else { return default };
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Null => return default,
        other => other.to_string(),
    };
    let cleaned: String = text.replace(',', "");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return default;
    }
    cleaned.parse::<f64>().map(|f| f as i64).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deep_get_navigates_mixed_string_and_index_keys() {
        let v = json!({"entities": {"urls": [{"expanded_url": "https://x"}]}});
        let got = dget!(&v, "entities", "urls", 0, "expanded_url");
        assert_eq!(got.and_then(|v| v.as_str()), Some("https://x"));
    }

    #[test]
    fn deep_get_returns_none_on_missing_path() {
        let v = json!({"a": {}});
        assert!(dget!(&v, "a", "b", "c").is_none());
    }

    #[test]
    fn parse_int_strips_commas_and_handles_float_strings() {
        assert_eq!(parse_int(Some(&json!("1,234")), 0), 1234);
        assert_eq!(parse_int(Some(&json!("12.0")), 0), 12);
        assert_eq!(parse_int(Some(&json!(" ")), 99), 99);
        assert_eq!(parse_int(None, 5), 5);
        assert_eq!(parse_int(Some(&json!(42)), 0), 42);
    }
}
