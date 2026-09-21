//! The agent-facing output envelope. See `PLAN.md` §5.1.
//!
//! Invariants (non-negotiable):
//! - stdout carries EXACTLY ONE machine-readable document per invocation
//!   (emitted only via [`emit`] — the single-print-owner discipline).
//! - Everything else (logs, warnings, progress) goes to stderr. This fixes
//!   the exact bug class upstream hit in its own issue #69, where a
//!   ClientTransaction-init warning corrupted JSON output by leaking into
//!   stdout. Test: `twr <any> --json | python -c json.load` always passes.
//!
//! Wire shape (plan §5.1): `{"ok":true,"schema_version":"1","type":...}` —
//! `ok` is a real boolean. (An earlier scaffold used serde's internally-tagged
//! `#[serde(tag = "ok")]` trick, which renders `ok` as the *string* `"true"`;
//! that was wrong per the contract and is fixed here with explicit structs.)

use serde::Serialize;

use crate::error::TwrError;

/// Pagination block shipped with list envelopes.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Pagination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

/// Meta block: trace id, command identity, counts, truncation flag.
#[derive(Debug, Clone, Serialize)]
pub struct Meta {
    #[serde(rename = "traceId")]
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "maxRequested"
    )]
    pub max_requested: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned: Option<usize>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "filterApplied"
    )]
    pub filter_applied: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl Meta {
    pub fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            command: None,
            max_requested: None,
            returned: None,
            filter_applied: None,
            truncated: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct SuccessBody<'a, T: Serialize> {
    ok: bool,
    schema_version: &'static str,
    #[serde(rename = "type")]
    kind: &'a str,
    data: &'a T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pagination: &'a Option<Pagination>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: &'a Option<Meta>,
}

#[derive(Debug, Serialize)]
struct ErrorBody<'a> {
    ok: bool,
    schema_version: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    error: &'a TwrError,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: &'a Option<Meta>,
}

/// Top-level envelope returned by every `twr` command in machine mode
/// (`--json` / `--yaml` / `--toon`).
#[derive(Debug)]
pub enum Envelope<T: Serialize> {
    Ok {
        kind: &'static str,
        data: T,
        pagination: Option<Pagination>,
        meta: Option<Meta>,
    },
    Err {
        error: TwrError,
        meta: Option<Meta>,
    },
}

impl<T: Serialize> Serialize for Envelope<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Envelope::Ok {
                kind,
                data,
                pagination,
                meta,
            } => SuccessBody {
                ok: true,
                schema_version: "1",
                kind,
                data,
                pagination,
                meta,
            }
            .serialize(s),
            Envelope::Err { error, meta } => ErrorBody {
                ok: false,
                schema_version: "1",
                kind: "error",
                error,
                meta,
            }
            .serialize(s),
        }
    }
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(kind: &'static str, data: T) -> Self {
        Envelope::Ok {
            kind,
            data,
            pagination: None,
            meta: None,
        }
    }

    pub fn err(error: TwrError) -> Self {
        Envelope::Err { error, meta: None }
    }

    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        if let Envelope::Ok {
            pagination: slot, ..
        } = &mut self
        {
            *slot = Some(pagination);
        }
        self
    }

    pub fn with_meta(mut self, meta: Meta) -> Self {
        match &mut self {
            Envelope::Ok { meta: slot, .. } | Envelope::Err { meta: slot, .. } => {
                *slot = Some(meta);
            }
        }
        self
    }
}

/// Render the envelope to its final stdout document.
/// THE single-print-owner: binary crates must print via this function and
/// must not `println!` the envelope (or anything else) anywhere else.
pub fn render_json<T: Serialize>(envelope: &Envelope<T>) -> String {
    serde_json::to_string(envelope).unwrap_or_else(|_| {
        r#"{"ok":false,"schema_version":"1","type":"error","error":{"code":"general-auth","message":"envelope serialization failed","suggestion":null,"retryable":false,"retry_after_ms":null,"failing_input":null}}"#.to_string()
    })
}

/// Emit the envelope to stdout — the ONE `println!` for machine output.
/// All diagnostics must use `eprintln!` / the log macros (stderr).
pub fn emit<T: Serialize>(envelope: &Envelope<T>) {
    println!("{}", render_json(envelope));
}

/// Apply `--compact`: strip `author.profile_image_url`, `media[].width` /
/// `media[].height`, and expanded `urls` — keeping
/// `id/text/author.screen_name/metrics/created_at` (+ `author.name` for
/// display). Operates on the serialized value so it works for any shape.
pub fn apply_compact(value: serde_json::Value) -> serde_json::Value {
    fn strip_tweet(tweet: &mut serde_json::Map<String, serde_json::Value>) {
        if let Some(author) = tweet.get_mut("author").and_then(|a| a.as_object_mut()) {
            author.remove("profile_image_url");
        }
        if let Some(media) = tweet.get_mut("media").and_then(|m| m.as_array_mut()) {
            for item in media.iter_mut().filter_map(|m| m.as_object_mut()) {
                item.remove("width");
                item.remove("height");
            }
        }
        tweet.remove("urls");
    }
    fn walk(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Array(items) => {
                for item in items.iter_mut() {
                    walk(item);
                }
            }
            serde_json::Value::Object(map) => {
                // Heuristic: an object with id+text+author is a tweet.
                if map.contains_key("id") && map.contains_key("text") && map.contains_key("author")
                {
                    strip_tweet(map);
                }
                for v in map.values_mut() {
                    walk(v);
                }
            }
            _ => {}
        }
    }
    let mut out = value;
    walk(&mut out);
    out
}

/// Apply `--fields a.b,c`: post-parse dotted-path projection over the
/// serialized data value. Unknown paths are skipped (not an error).
///
/// Collection envelopes (`tweet_list`, `user_list`, …) are projected
/// per-item, not at the envelope root: `--fields id,text` on a
/// `tweet_list` projects each tweet in `data.tweets` (live-found
/// 2026-09-21: root-level lookup returned `{}` because the envelope root
/// has no `id`/`text` keys). Pass `tweets.id` only when you want the
/// envelope shape preserved around the projected items.
pub fn apply_fields(data: &serde_json::Value, fields: &[String]) -> serde_json::Value {
    fn get_path(value: &serde_json::Value, path: &[&str]) -> Option<serde_json::Value> {
        // Intermediate arrays map the remainder over each item
        // (`tweets.id` on `{tweets: [{id..}]}` → `[{id..}]`), so dotted
        // paths keep working on collection envelopes.
        if path.is_empty() {
            return Some(value.clone());
        }
        match value {
            serde_json::Value::Array(items) => {
                let mapped: Vec<serde_json::Value> =
                    items.iter().filter_map(|it| get_path(it, path)).collect();
                if mapped.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Array(mapped))
                }
            }
            serde_json::Value::Object(_) => {
                let next = value.get(path[0])?;
                get_path(next, &path[1..])
            }
            _ => None,
        }
    }
    fn set_path(
        map: &mut serde_json::Map<String, serde_json::Value>,
        path: &[&str],
        v: serde_json::Value,
    ) {
        if path.is_empty() {
            return;
        }
        if path.len() == 1 {
            map.insert(path[0].to_string(), v);
            return;
        }
        let entry = map
            .entry(path[0].to_string())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let Some(obj) = entry.as_object_mut() {
            set_path(obj, &path[1..], v);
        }
    }
    if data.is_array() {
        return serde_json::Value::Array(
            data.as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|item| apply_fields(item, fields).clone())
                        .collect()
                })
                .unwrap_or_default(),
        );
    }
    // Collection envelopes carry items under a single array key
    // (`tweets`, `users`, `lists`, …): project each item so bare field
    // names (`--fields id,text`) work without forcing callers to spell
    // the envelope key (`tweets.id`). Explicit `tweets.id` paths keep
    // working — they resolve against the envelope directly first.
    if let serde_json::Value::Object(map) = data {
        let arrays: Vec<(&String, &Vec<serde_json::Value>)> = map
            .iter()
            .filter_map(|(k, v)| v.as_array().map(|a| (k, a)))
            .collect();
        if arrays.len() == 1 {
            let (key, items) = arrays[0];
            if fields.iter().all(|f| !f.contains('.')) {
                let projected: Vec<serde_json::Value> = items
                    .iter()
                    .map(|item| apply_fields(item, fields).clone())
                    .collect();
                let mut out = serde_json::Map::new();
                out.insert(key.clone(), serde_json::Value::Array(projected));
                return serde_json::Value::Object(out);
            }
        }
    }
    let mut out = serde_json::Map::new();
    for field in fields {
        let path: Vec<&str> = field.split('.').collect();
        if let Some(v) = get_path(data, &path) {
            set_path(&mut out, &path, v.clone());
        }
    }
    serde_json::Value::Object(out)
}

/// Parse a `--fields` CSV into paths.
pub fn parse_fields(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ok_envelope_serializes_with_schema_version_1() {
        let env = Envelope::ok("tweet_list", json!([1, 2]));
        let raw = render_json(&env);
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        // Contract: ok is a real boolean, not the string "true".
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["schema_version"], json!("1"));
        assert_eq!(v["type"], json!("tweet_list"));
    }

    #[test]
    fn err_envelope_has_type_error_and_meta() {
        let err = TwrError::new(crate::ErrorKind::NotFound, "gone");
        let env: Envelope<serde_json::Value> = Envelope::err(err).with_meta(Meta::new("trace-1"));
        let raw = render_json(&env);
        // The stdout-invariants: exactly one document, always parseable.
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["type"], json!("error"));
        assert_eq!(v["meta"]["traceId"], json!("trace-1"));
    }

    #[test]
    fn compact_strips_images_dims_and_urls() {
        let data = json!([{
            "id": "1", "text": "hi",
            "author": {"screen_name": "a", "name": "A", "profile_image_url": "http://x"},
            "media": [{"type": "photo", "url": "u", "width": 1, "height": 2}],
            "urls": ["http://e"], "metrics": {}, "created_at": "t",
        }]);
        let out = apply_compact(data);
        let t = &out[0];
        assert!(t["author"].get("profile_image_url").is_none());
        assert_eq!(t["author"]["screen_name"], json!("a"));
        assert!(t.get("urls").is_none());
        assert!(t["media"][0].get("width").is_none());
        assert_eq!(t["id"], json!("1"));
    }

    #[test]
    fn fields_projects_dotted_paths() {
        let data = json!({"id": "1", "text": "hi", "author": {"screen_name": "a", "extra": 1}});
        let out = apply_fields(&data, &["id".into(), "author.screen_name".into()]);
        assert_eq!(out, json!({"id": "1", "author": {"screen_name": "a"}}));
        let arr = apply_fields(&json!([data]), &["id".into()]);
        assert_eq!(arr, json!([{"id": "1"}]));
    }

    #[test]
    fn fields_project_each_item_of_collection_envelopes() {
        // Live-found 2026-09-21: `--fields id,text` on a tweet_list
        // returned `{}` (root has no id/text). Bare names now project
        // per item under the single array key.
        let data = json!({"tweets": [{"id": "1", "text": "hi", "extra": 9}]});
        let out = apply_fields(&data, &["id".into(), "text".into()]);
        assert_eq!(out, json!({"tweets": [{"id": "1", "text": "hi"}]}));
        // Dotted paths keep resolving against the envelope directly.
        let out2 = apply_fields(&data, &["tweets.id".into()]);
        assert!(out2.get("tweets").is_some());
    }

    #[test]
    fn parse_fields_splits_csv() {
        assert_eq!(
            parse_fields("id, text ,author.screen_name"),
            vec!["id", "text", "author.screen_name"]
        );
        assert!(parse_fields("").is_empty());
    }
}
