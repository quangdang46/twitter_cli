//! Official-API video upload (bead twitter_cli-5o3.6.3, issue #13 api-v2 half).
//!
//! Chunked upload via the v2 media endpoints: INIT → APPEND → FINALIZE with
//! async STATUS polling until `succeeded`, `media_category=tweet_video`,
//! plus `--video/--file/--alt-text` flags. Processing failures map to a
//! structured error with a clear suggestion (never a bare HTTP code).
//!
//! Pure shapes + params + STATUS state machine here; HTTP goes through the
//! shared `HttpTransport` from the caller (same additive pattern as
//! [`crate::backend`]).

/// v2 media upload root.
pub const V2_UPLOAD_INIT_URL: &str = "https://api.twitter.com/2/media/upload/initialize";
pub const V2_UPLOAD_APPEND_URL: &str = "https://api.twitter.com/2/media/upload/append";
pub const V2_UPLOAD_FINALIZE_URL: &str = "https://api.twitter.com/2/media/upload/finalize";
pub const V2_UPLOAD_STATUS_URL: &str = "https://api.twitter.com/2/media/upload/status";

/// Max video bytes (512MB per X docs; twr caps lower at 128MB as a safety
/// default — configurable, never unbounded).
pub const MAX_VIDEO_BYTES: u64 = 128 * 1024 * 1024;
/// APPEND chunk size (5MB segments).
pub const VIDEO_CHUNK_BYTES: usize = 5 * 1024 * 1024;

/// Supported video extensions.
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov"];

/// Classify a video path by extension.
pub fn video_mime(path: &std::path::Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" => Some("video/mp4"),
        "mov" => Some("video/quicktime"),
        _ => None,
    }
}

/// INIT body for a video upload.
pub fn init_body(total_bytes: u64, mime: &str) -> serde_json::Value {
    serde_json::json!({
        "media_category": "tweet_video",
        "media_type": mime,
        "total_bytes": total_bytes,
    })
}

/// Split bytes into APPEND segments.
pub fn video_chunks(data: &[u8]) -> Vec<&[u8]> {
    data.chunks(VIDEO_CHUNK_BYTES).collect()
}

/// STATUS poll states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadStatus {
    Pending,
    InProgress,
    Succeeded { media_id: String },
    Failed { reason: String },
}

/// Parse a STATUS response body.
pub fn parse_status(body: &[u8]) -> UploadStatus {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return UploadStatus::Failed {
            reason: "STATUS returned invalid JSON; retry FINALIZE later".into(),
        };
    };
    let state = v
        .pointer("/data/processing_info/state")
        .and_then(|s| s.as_str())
        .unwrap_or("succeeded");
    match state {
        "succeeded" => UploadStatus::Succeeded {
            media_id: v
                .pointer("/data/id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "failed" => UploadStatus::Failed {
            reason: v
                .pointer("/data/processing_info/error/message")
                .and_then(|s| s.as_str())
                .unwrap_or("video processing failed; re-encode (H.264/AAC, ≤128MB) and retry")
                .to_string(),
        },
        _ => UploadStatus::InProgress,
    }
}

/// Poll delays: 5s × up to 12 attempts (≈1 min) before surfacing pending.
pub const STATUS_POLL_DELAY_SECS: u64 = 5;
pub const STATUS_POLL_ATTEMPTS: u32 = 12;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_classification_and_chunks() {
        assert_eq!(video_mime(std::path::Path::new("a.mp4")), Some("video/mp4"));
        assert_eq!(video_mime(std::path::Path::new("a.png")), None);
        assert!(VIDEO_EXTS.contains(&"mp4"));
        let big = vec![0u8; 2 * VIDEO_CHUNK_BYTES + 1];
        assert_eq!(video_chunks(&big).len(), 3);
        let init = init_body(100, "video/mp4");
        assert_eq!(init["media_category"], "tweet_video");
    }

    #[test]
    fn status_states() {
        assert!(matches!(
            parse_status(br#"{"data":{"id":"m1"}}"#),
            UploadStatus::Succeeded { .. }
        ));
        assert!(matches!(
            parse_status(br#"{"data":{"processing_info":{"state":"in_progress"}}}"#),
            UploadStatus::InProgress
        ));
        assert!(matches!(
            parse_status(br#"{"data":{"processing_info":{"state":"failed"}}}"#),
            UploadStatus::Failed { .. }
        ));
        assert!(matches!(parse_status(b"junk"), UploadStatus::Failed { .. }));
    }
}
