//! Write-command execution: gate → idempotency → transport → parse.
//!
//! Shared by post/reply/quote (3.4.3) and the engagement/management writes
//! (3.4.4). Every write flows through:
//! 1. `twr_core::decide` (§5.3 table) — Preview/Execute/Prompt/Deny.
//! 2. Idempotency pre-check (replay cached / refuse unknown).
//! 3. GraphQL POST (or 1.1 friendships form-POST for follow/unfollow).
//! 4. `_write_delay` jitter (1.5–4s, mirrors Python).
//!
//! Media upload itself is bead 3.4.5 — here `-i` paths are validated
//! (exist + ≤5MB + jpeg/png/gif/webp) and recorded as pending; the actual
//! INIT→APPEND→FINALIZE lands with the upload bead.

use twr_core::{decide, ApplyInput};

/// Max attached images per post (plan §1.1).
pub const MAX_IMAGES: usize = 4;
/// Max image bytes (5MB; chunked GIF to 15MB lands in 3.4.5).
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp"];

/// Validate `-i` image paths without uploading. Returns the paths when all
/// exist, are files, have image extensions, and fit in 5MB.
pub fn validate_images(paths: &[String]) -> Result<Vec<String>, String> {
    if paths.len() > MAX_IMAGES {
        return Err(format!(
            "at most {MAX_IMAGES} images per post (got {})",
            paths.len()
        ));
    }
    for p in paths {
        let path = std::path::Path::new(p);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !IMAGE_EXTS.contains(&ext.as_str()) {
            return Err(format!("unsupported image type: {p} (jpeg/png/gif/webp)"));
        }
        let meta = std::fs::metadata(path).map_err(|_| format!("cannot read image file: {p}"))?;
        if !meta.is_file() {
            return Err(format!("not a file: {p}"));
        }
        if meta.len() > MAX_IMAGE_BYTES {
            return Err(format!("image over 5MB: {p} (chunked GIF lands in 3.4.5)"));
        }
    }
    Ok(paths.to_vec())
}

/// Variables for CreateTweet (post / reply / quote / edit share the op).
/// `edit_target`: Some(id) adds `edit_options: {previous_tweet_id}` —
/// the twikit mechanism (same op + endpoint, NOT a separate EditTweet
/// mutation; see bead o1l.4.2 comment). `card_uri`: poll card passthrough
/// (twikit `poll_uri` → `card_uri`; bead o1l.4.3 comment).
/// Thin wrapper kept for the existing unit test + any plain-path callers.
#[allow(dead_code)]
pub fn create_tweet_vars(
    text: &str,
    reply_to: Option<&str>,
    quote_url: Option<&str>,
    media_ids: &[String],
) -> serde_json::Value {
    create_tweet_vars_full(text, reply_to, quote_url, media_ids, None, None)
}

/// Full variant with edit + card_uri (edit/poll beads).
pub fn create_tweet_vars_full(
    text: &str,
    reply_to: Option<&str>,
    quote_url: Option<&str>,
    media_ids: &[String],
    card_uri: Option<&str>,
    edit_target: Option<&str>,
) -> serde_json::Value {
    let media_entities: Vec<serde_json::Value> = media_ids
        .iter()
        .map(|mid| serde_json::json!({"media_id": mid, "tagged_users": []}))
        .collect();
    let mut vars = serde_json::json!({
        "tweet_text": text,
        "media": {"media_entities": media_entities, "possibly_sensitive": false},
        "semantic_annotation_ids": [],
        "dark_request": false,
    });
    if let Some(reply_id) = reply_to {
        vars["reply"] = serde_json::json!({
            "in_reply_to_tweet_id": reply_id,
            "exclude_reply_user_ids": [],
        });
    }
    if let Some(url) = quote_url {
        vars["attachment_url"] = serde_json::json!(url);
    }
    if let Some(uri) = card_uri {
        vars["card_uri"] = serde_json::json!(uri);
    }
    if let Some(prev) = edit_target {
        vars["edit_options"] = serde_json::json!({"previous_tweet_id": prev});
    }
    vars
}

/// Variables for PinTweet/UnpinTweet: `{"tweet_id"}` (deck GraphQL.json
/// capture; same bare shape as the favorite/bookmark ops).
pub fn pin_vars(tweet_id: &str) -> serde_json::Value {
    serde_json::json!({"tweet_id": tweet_id})
}

/// Standard-post weighted-length threshold: X's rule is 280 weighted
/// characters (twitter-text weighting: URLs count 23 chars regardless of
/// length). twr does NOT vendor twitter-text: this counts Unicode scalar
/// values with URL normalization (http/https links → 23), which matches
/// twitter-text v3 for the cases that matter here (ASCII + CJK + emoji +
/// URLs) and can only differ on exotic-case weights. Over-threshold text
/// routes to CreateNoteTweet; at-or-under stays on CreateTweet (mirrors
/// upstream PR #64's routing test). Conservative direction: a false
/// "over" only sends a Premium-gated op that fails cleanly with exit 4,
/// while a false "under" would 186-fail a post that should have routed.
pub const NOTE_TWEET_THRESHOLD: usize = 280;

/// Weighted length of tweet text: URLs → 23 chars each, everything else →
/// one unit per Unicode scalar value.
pub fn weighted_length(text: &str) -> usize {
    let mut len = 0;
    let mut rest = text;
    while let Some(pos) = rest.find("http://").or_else(|| rest.find("https://")) {
        len += rest[..pos].chars().count();
        let after = &rest[pos..];
        let url_len = after
            .split_whitespace()
            .next()
            .map(|u| {
                // Trim trailing punctuation the counter wouldn't include.
                u.trim_end_matches([',', '.', '!', '?', ';', ':', ')', ']', '"', '\''])
                    .len()
            })
            .unwrap_or(0);
        len += 23;
        rest = &after[url_len.min(after.len())..];
    }
    len + rest.chars().count()
}

/// True when text must route to CreateNoteTweet (over-threshold).
/// Reply/quote bodies use the SAME threshold (upstream PR #64's lesson:
/// quote_tweet must route too, not just create_tweet).
pub fn needs_note_tweet(text: &str) -> bool {
    weighted_length(text) > NOTE_TWEET_THRESHOLD
}

/// Variables for CreateNoteTweet: 1-1 from Rettiwt-API `postNote` VERBATIM
/// (pinned blob 4f11105, src/requests/Tweet.ts; full body in bead
/// o1l.4.1's comment; independently fetched + verified by c1).
/// `disallowed_reply_options: null` is LOAD-BEARING (omitting it is the
/// silent-empty-tweet_results failure mode). NO `dark_request` (CreateTweet
/// has it, NoteTweet does not). `media`: MediaVariable-shaped when images
/// exist, else UNDEFINED (Rust: omit the key — `None`, not null).
/// NO `queryId`-in-body, NO `fieldToggles` (do NOT copy the list-mutation
/// or CreateTweet patterns here). Reply/quote long-form vars UNCONFIRMED
/// (postNote has no reply/quote path) — the router sends plain-post shape
/// only; long quote/reply FAILS CLOSED below instead of guessing.
pub fn note_tweet_vars(
    text: &str,
    media_ids: &[String],
    reply_to: Option<&str>,
    quote_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    if reply_to.is_some() || quote_id.is_some() {
        return Err(
            "long-form reply/quote variables are UNCONFIRMED (Rettiwt postNote has no reply/quote path) — refusing to guess; shorten the text or drop the reply/quote target"
                .to_string(),
        );
    }
    let mut vars = serde_json::json!({
        "tweet_text": text,
        "semantic_annotation_ids": [],
        "disallowed_reply_options": null,
    });
    if !media_ids.is_empty() {
        let entities: Vec<serde_json::Value> = media_ids
            .iter()
            .map(|mid| serde_json::json!({"media_id": mid, "tagged_users": []}))
            .collect();
        vars["media"] = serde_json::json!({
            "media_entities": entities,
            "possibly_sensitive": false,
        });
    }
    Ok(vars)
}

/// Classify an edit rejection message into (code, suggestion), all
/// exit-4 / retryable:false (o1l.4.2 taxonomy). Message-substring based —
/// X has no stable numeric code per case. Returns None when the message
/// matches none of the three known cases (caller falls through to the
/// generic from_api_code path).
pub fn classify_edit_rejection(message: &str) -> Option<(&'static str, &'static str)> {
    let m = message.to_lowercase();
    if m.contains("not eligible")
        || m.contains("premium")
        || m.contains("blue")
        || m.contains("subscri")
    {
        Some((
            "edit-not-eligible",
            "tweet editing needs X Premium on this account; delete-and-repost instead, or give up",
        ))
    } else if m.contains("window")
        || m.contains("expired")
        || m.contains("too old")
        || m.contains("editable_until")
    {
        Some((
            "edit-window-expired",
            "the edit window for this tweet has passed; delete-and-repost instead — retrying will never succeed",
        ))
    } else if m.contains("count")
        || m.contains("exhausted")
        || m.contains("limit")
        || m.contains("no more edits")
        || m.contains("locked")
    {
        Some((
            "edit-count-exhausted",
            "this tweet has no edits remaining; delete-and-repost instead — retrying will never succeed",
        ))
    } else {
        None
    }
}

/// Parse a CreateNoteTweet response's tweet id across the three envelope
/// shapes seen in the wild (`create_tweet` / `notetweet_create` /
/// `create_note_tweet`). Returns None when NO shape confirms — the caller
/// FAILS CLOSED (explicit error, never false-success; PR #65 lesson).
pub fn parse_note_tweet_id(payload: &serde_json::Value) -> Option<String> {
    for pointer in [
        "/data/create_tweet/tweet_results/result/rest_id",
        "/data/notetweet_create/notetweet_results/result/rest_id",
        "/data/notetweet_create/tweet_results/result/rest_id",
        "/data/create_note_tweet/tweet_results/result/rest_id",
        "/data/create_note_tweet/notetweet_results/result/rest_id",
    ] {
        if let Some(id) = payload
            .pointer(pointer)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return Some(id.to_string());
        }
    }
    None
}

/// Variables for CreateList: `{name, description?, isPrivate?}`.
/// Source: Rettiwt-API `List.create` VERBATIM (pinned blob 4f11105,
/// src/requests/List.ts — independently fetched by c1 at
/// cdn.jsdelivr.net/npm/rettiwt-api@7.1.3/src/requests/List.ts).
/// The "cephalochromoscope + emusks" citations previously attached here
/// were web-search snippets WITHOUT a local file — removed per the
/// path-or-UNCONFIRMED rule. (The live 214's cause is still UNRESOLVED:
/// vars keys match Rettiwt exactly, so the 214 is NOT a vars-keys issue —
/// see hypotheses in `run_list_write`; features-override and
/// queryId-in-body fixes both failed live.)
/// `description` omitted when empty (Rettiwt conditional-spread verbatim).
pub fn create_list_vars(name: &str, description: Option<&str>, private: bool) -> serde_json::Value {
    let mut vars = serde_json::json!({"name": name, "isPrivate": private});
    if let Some(d) = description.filter(|d| !d.is_empty()) {
        vars["description"] = serde_json::json!(d);
    }
    vars
}

/// Variables for UpdateList: SPARSE partial update (`{listId}` + only the
/// fields being changed). Confirmed by Rettiwt-API `List.update`
/// (live-shaped: `{listId, ...(isPrivate?), ...(description?),
/// ...(name?)}` — each field conditionally spread, never full-resend).
/// The CLI still requires all three flags (full-resend at the CLI layer —
/// never send a sparse shape that could blank a field server-side by
/// accident), but the WIRE shape is per-field conditional like Rettiwt's.
pub fn update_list_vars(
    list_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    private: Option<bool>,
) -> serde_json::Value {
    let mut vars = serde_json::json!({"listId": list_id});
    if let Some(p) = private {
        vars["isPrivate"] = serde_json::json!(p);
    }
    if let Some(d) = description {
        vars["description"] = serde_json::json!(d);
    }
    if let Some(n) = name {
        vars["name"] = serde_json::json!(n);
    }
    vars
}

/// Variables for DeleteList / ListSubscribe / ListUnsubscribe /
/// UpdatePinnedTimelines-by-id: bare `{listId}` (deck op names confirmed;
/// exact key casing `listId` follows every other list op's convention in
/// bird/xfetch — live-verify on first real call).
pub fn list_id_vars(list_id: &str) -> serde_json::Value {
    serde_json::json!({"listId": list_id})
}

/// Variables for ListAddMember/ListRemoveMember: `{listId, userId}` — ONE
/// user per invocation, no batch flag (ban-risk rule, plan §13.3: bulk
/// list-adds are a spam-report vector; looping belongs in the orchestrator,
/// not in twr).
pub fn list_member_vars(list_id: &str, user_id: &str) -> serde_json::Value {
    serde_json::json!({"listId": list_id, "userId": user_id})
}

/// Per-op `variables` for the engagement ops, mirroring the Python
/// `client.py` shapes exactly (live-fixed 2026-09-16): `DeleteRetweet`
/// takes `{"source_tweet_id"}` NOT `{"tweet_id"}` (a real 400-class server
/// rejection mis-surfaced as exit-6 otherwise); `CreateRetweet` and the
/// *unlike* path take `{"tweet_id","dark_request":false}`; plain
/// favorite/bookmark ops take a bare `{"tweet_id"}`.
pub fn tweet_id_vars(op: &str, tweet_id: &str) -> serde_json::Value {
    match op {
        "DeleteRetweet" => serde_json::json!({"source_tweet_id": tweet_id, "dark_request": false}),
        "CreateRetweet" | "DeleteTweet" | "UnfavoriteTweet" => {
            serde_json::json!({"tweet_id": tweet_id, "dark_request": false})
        }
        _ => serde_json::json!({"tweet_id": tweet_id}),
    }
}

/// Jittered post-write delay bounds (mirrors `_write_delay` 1.5–4s).
/// `u01` is a caller-supplied uniform in [0,1) so tests can pin it.
pub fn write_delay_secs(u01: f64) -> f64 {
    1.5 + 2.5 * u01.clamp(0.0, 1.0)
}

/// Resolve the §5.3 gate for a write invocation. `prompt_yes` is the TTY
/// answer when the table says Prompt (true=yes→execute). Returns the
/// [`twr_core::Decision`] so callers branch uniformly.
pub fn gate(
    apply: bool,
    dry_run: bool,
    no_interactive: bool,
    stdin_is_tty: bool,
) -> twr_core::Decision {
    decide(&ApplyInput {
        apply,
        dry_run,
        no_interactive,
        stdin_is_tty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use twr_core::Decision;

    #[test]
    fn image_validation_rejects_count_type_and_size() {
        assert!(validate_images(&[]).unwrap().is_empty());
        let many: Vec<String> = (0..5).map(|i| format!("{i}.jpg")).collect();
        assert!(validate_images(&many).is_err());
        assert!(validate_images(&["x.bmp".into()]).is_err());
        assert!(validate_images(&["missing.jpg".into()]).is_err());
    }

    #[test]
    fn image_validation_accepts_a_real_small_png() {
        let dir = std::env::temp_dir().join(format!("twr-img-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.png");
        std::fs::write(&p, [0x89, b'P', b'N', b'G']).unwrap();
        let ok = validate_images(&[p.display().to_string()]).unwrap();
        assert_eq!(ok.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_tweet_vars_shapes() {
        let v = create_tweet_vars("hi", None, None, &[]);
        assert_eq!(v["tweet_text"], "hi");
        assert!(v.get("reply").is_none());
        let r = create_tweet_vars("hi", Some("123"), None, &[]);
        assert_eq!(r["reply"]["in_reply_to_tweet_id"], "123");
        let q = create_tweet_vars("c", None, Some("https://x.com/i/status/9"), &[]);
        assert_eq!(q["attachment_url"], "https://x.com/i/status/9");
    }

    #[test]
    fn write_delay_stays_in_1_5_to_4s() {
        assert_eq!(write_delay_secs(0.0), 1.5);
        assert_eq!(write_delay_secs(1.0), 4.0);
    }

    #[test]
    fn gate_delegates_to_decision_table() {
        assert_eq!(gate(true, false, false, true), Decision::Execute);
        assert_eq!(gate(false, true, false, true), Decision::Preview);
    }

    #[test]
    fn pin_vars_are_bare_tweet_id() {
        let v = pin_vars("123");
        assert_eq!(v["tweet_id"], "123");
        assert!(v.get("dark_request").is_none());
    }

    #[test]
    fn weighted_length_counts_urls_as_23_and_chars_otherwise() {
        assert_eq!(weighted_length("hello"), 5);
        assert_eq!(weighted_length(""), 0);
        // URL normalization: a 40-char link counts 23.
        let with_url = "see https://x.com/some/very/long/path/1234567890 end";
        assert_eq!(
            weighted_length(with_url),
            4 + 23 + 4,
            "4 chars + url(23) + 4 chars"
        );
        assert!(!needs_note_tweet("short"));
        assert!(needs_note_tweet(&"x".repeat(281)));
        assert!(!needs_note_tweet(&"x".repeat(280)));
    }

    #[test]
    fn note_tweet_vars_carry_disallowed_reply_options_and_no_dark_request() {
        let v = note_tweet_vars("long text", &[], None, None).unwrap();
        assert_eq!(v["tweet_text"], "long text");
        assert!(v.get("disallowed_reply_options").is_some());
        assert!(v.get("dark_request").is_none());
        assert!(
            v.get("media").is_none(),
            "no media key when empty (undefined, not null)"
        );
        assert!(v.get("queryId").is_none());
        assert!(v.get("fieldToggles").is_none());
        let m = note_tweet_vars("t", &["m1".to_string()], None, None).unwrap();
        assert_eq!(m["media"]["media_entities"][0]["media_id"], "m1");
    }

    #[test]
    fn note_tweet_vars_fail_closed_on_reply_and_quote() {
        assert!(note_tweet_vars("t", &[], Some("123"), None).is_err());
        assert!(note_tweet_vars("t", &[], None, Some("456")).is_err());
    }

    #[test]
    fn edit_rejection_taxonomy_has_three_distinct_cases() {
        let (c1, _) =
            classify_edit_rejection("not eligible for editing, Premium required").unwrap();
        let (c2, _) = classify_edit_rejection("edit window expired").unwrap();
        let (c3, _) = classify_edit_rejection("edit count exhausted, locked").unwrap();
        assert_eq!(
            (c1, c2, c3),
            (
                "edit-not-eligible",
                "edit-window-expired",
                "edit-count-exhausted"
            )
        );
        assert!(classify_edit_rejection("something entirely different").is_none());
    }

    #[test]
    fn parse_note_tweet_id_covers_all_three_envelope_shapes() {
        let a = serde_json::json!({"data": {"create_tweet": {"tweet_results": {"result": {"rest_id": "1"}}}}});
        let b = serde_json::json!({"data": {"notetweet_create": {"notetweet_results": {"result": {"rest_id": "2"}}}}});
        let c = serde_json::json!({"data": {"create_note_tweet": {"tweet_results": {"result": {"rest_id": "3"}}}}});
        assert_eq!(parse_note_tweet_id(&a).as_deref(), Some("1"));
        assert_eq!(parse_note_tweet_id(&b).as_deref(), Some("2"));
        assert_eq!(parse_note_tweet_id(&c).as_deref(), Some("3"));
        assert!(parse_note_tweet_id(&serde_json::json!({"data": {}})).is_none());
    }

    #[test]
    fn list_vars_shapes() {
        let c = create_list_vars("Rust", Some("desc"), true);
        assert_eq!(c["name"], "Rust");
        assert_eq!(c["description"], "desc");
        assert_eq!(c["isPrivate"], true);
        let c2 = create_list_vars("N", None, false);
        assert!(c2.get("description").is_none());
        let u = update_list_vars("L1", Some("N"), Some("D"), Some(false));
        assert_eq!(u["listId"], "L1");
        assert_eq!(u["name"], "N");
        assert_eq!(list_id_vars("L1")["listId"], "L1");
        let m = list_member_vars("L1", "U9");
        assert_eq!(m["listId"], "L1");
        assert_eq!(m["userId"], "U9");
    }
}
