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

/// Simple `{"tweet_id": …}` / `{"tweet_id","dark_request"}` variables for the
/// engagement ops (favorite/retweet/bookmark/delete + their reverses).
/// Consumed by bead 3.4.4; kept alive here so the shape is reviewed once.
#[allow(dead_code)]
pub fn tweet_id_vars(op: &str, tweet_id: &str) -> serde_json::Value {
    match op {
        "CreateRetweet" | "DeleteTweet" => {
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
}
