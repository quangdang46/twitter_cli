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

/// Variables for CreateTweet (post / reply / quote share the op).
pub fn create_tweet_vars(
    text: &str,
    reply_to: Option<&str>,
    quote_url: Option<&str>,
    media_ids: &[String],
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
    vars
}

/// Variables for PinTweet/UnpinTweet: `{"tweet_id"}` (deck GraphQL.json
/// capture; same bare shape as the favorite/bookmark ops).
pub fn pin_vars(tweet_id: &str) -> serde_json::Value {
    serde_json::json!({"tweet_id": tweet_id})
}

/// Variables for CreateList: `{name, description?, isPrivate?}` (op + ID
/// confirmed in docs/json/API.json; this shape REJECTED live 2026-09-16
/// with X code 214 DecodeException — exact accepted keys UNKNOWN.
/// The sibling 1.1-style `{"list_id","name","mode","description"}` shape is
/// the next candidate (mode=public|private instead of isPrivate bool).
/// c1: confirm live with the alternate shape before trusting this one.
/// X validates mutation variables strictly — a 214 means shape, not auth.
pub fn create_list_vars(name: &str, description: Option<&str>, private: bool) -> serde_json::Value {
    let mut vars = serde_json::json!({"name": name, "isPrivate": private});
    if let Some(d) = description.filter(|d| !d.is_empty()) {
        vars["description"] = serde_json::json!(d);
    }
    vars
}

/// Variables for UpdateList: full-field resend (`{listId, name,
/// description, isPrivate}`) — partial-update semantics UNCONFIRMED in the
/// corpus, so resend all three (c1: confirm live whether omitting unchanged
/// fields works; until then never send a sparse shape that could blank a
/// field server-side).
pub fn update_list_vars(
    list_id: &str,
    name: &str,
    description: &str,
    private: bool,
) -> serde_json::Value {
    serde_json::json!({
        "listId": list_id,
        "name": name,
        "description": description,
        "isPrivate": private,
    })
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
    fn list_vars_shapes() {
        let c = create_list_vars("Rust", Some("desc"), true);
        assert_eq!(c["name"], "Rust");
        assert_eq!(c["description"], "desc");
        assert_eq!(c["isPrivate"], true);
        let c2 = create_list_vars("N", None, false);
        assert!(c2.get("description").is_none());
        let u = update_list_vars("L1", "N", "D", false);
        assert_eq!(u["listId"], "L1");
        assert_eq!(u["name"], "N");
        assert_eq!(list_id_vars("L1")["listId"], "L1");
        let m = list_member_vars("L1", "U9");
        assert_eq!(m["listId"], "L1");
        assert_eq!(m["userId"], "U9");
    }
}
