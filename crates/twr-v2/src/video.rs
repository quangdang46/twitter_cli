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

/// v2 media upload root (bead 93i): APPEND/FINALIZE carry `media_id` in
/// the path; STATUS is a GET with `command=STATUS&media_id=…` query.
pub const V2_UPLOAD_INIT_URL: &str = "https://api.twitter.com/2/media/upload/initialize";
const V2_UPLOAD_ROOT: &str = "https://api.twitter.com/2/media/upload";

/// APPEND endpoint: `POST /2/media/upload/{media_id}/append`.
pub fn append_url(media_id: &str) -> String {
    format!("{V2_UPLOAD_ROOT}/{media_id}/append")
}

/// FINALIZE endpoint: `POST /2/media/upload/{media_id}/finalize`.
pub fn finalize_url(media_id: &str) -> String {
    format!("{V2_UPLOAD_ROOT}/{media_id}/finalize")
}

/// STATUS endpoint: `GET /2/media/upload?command=STATUS&media_id=…`.
pub fn status_url(media_id: &str) -> String {
    format!("{V2_UPLOAD_ROOT}?command=STATUS&media_id={media_id}")
}

/// Build the raw multipart APPEND body (bead 93i): the media bytes go in
/// the `media` file field verbatim — NOT base64-in-JSON. `boundary` is
/// caller-supplied so tests can pin it.
pub fn append_body(
    media_id: &str,
    segment_index: usize,
    segment: &[u8],
    boundary: &str,
) -> Vec<u8> {
    let mut parts: Vec<u8> = Vec::new();
    let field = |name: &str, value: &str| {
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
    };
    parts.extend_from_slice(field("media_id", media_id).as_bytes());
    parts.extend_from_slice(field("segment_index", &segment_index.to_string()).as_bytes());
    parts.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"media\"; filename=\"media\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
    );
    parts.extend_from_slice(segment);
    parts.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    parts
}

/// The `Content-Type` header value matching [`append_body`]'s boundary.
pub fn append_content_type(boundary: &str) -> String {
    format!("multipart/form-data; boundary={boundary}")
}

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
    // Only an EXPLICIT `succeeded` state counts (bead or4): missing
    // `processing_info` or a missing `state` key means "still working" —
    // keep polling — never a false success.
    let state = v
        .pointer("/data/processing_info/state")
        .and_then(|s| s.as_str());
    match state {
        Some("succeeded") => UploadStatus::Succeeded {
            media_id: v
                .pointer("/data/id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        },
        Some("failed") => UploadStatus::Failed {
            reason: v
                .pointer("/data/processing_info/error/message")
                .and_then(|s| s.as_str())
                .unwrap_or("video processing failed; re-encode (H.264/AAC, ≤128MB) and retry")
                .to_string(),
        },
        Some("pending") => UploadStatus::Pending,
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
    fn upload_endpoints_carry_media_id() {
        // Bead 93i: append/finalize paths include media_id; status is a
        // GET query, not a POST body.
        assert_eq!(
            append_url("m123"),
            "https://api.twitter.com/2/media/upload/m123/append"
        );
        assert_eq!(
            finalize_url("m123"),
            "https://api.twitter.com/2/media/upload/m123/finalize"
        );
        assert_eq!(
            status_url("m123"),
            "https://api.twitter.com/2/media/upload?command=STATUS&media_id=m123"
        );
    }

    #[test]
    fn append_body_is_raw_multipart() {
        // Bead 93i: raw bytes in the `media` part, no base64, no JSON.
        let seg = b"\x00\xffrawbytes";
        let body = append_body("m1", 2, seg, "bnd");
        let s = String::from_utf8_lossy(&body);
        assert!(s.contains("name=\"media_id\"\r\n\r\nm1"));
        assert!(s.contains("name=\"segment_index\"\r\n\r\n2"));
        assert!(s.contains("name=\"media\"; filename=\"media\""));
        assert!(body.windows(seg.len()).any(|w| w == seg));
        assert!(!s.contains("base64"));
        assert_eq!(
            append_content_type("bnd"),
            "multipart/form-data; boundary=bnd"
        );
    }

    #[test]
    fn status_states() {
        assert!(matches!(
            parse_status(br#"{"data":{"id":"m1","processing_info":{"state":"succeeded"}}}"#),
            UploadStatus::Succeeded { .. }
        ));
        assert!(matches!(
            parse_status(br#"{"data":{"processing_info":{"state":"in_progress"}}}"#),
            UploadStatus::InProgress
        ));
        assert!(matches!(
            parse_status(br#"{"data":{"processing_info":{"state":"pending"}}}"#),
            UploadStatus::Pending
        ));
        assert!(matches!(
            parse_status(br#"{"data":{"processing_info":{"state":"failed"}}}"#),
            UploadStatus::Failed { .. }
        ));
        assert!(matches!(parse_status(b"junk"), UploadStatus::Failed { .. }));
        // Bead or4: no explicit `succeeded` state is NEVER success — a bare
        // `data.id`, missing processing_info, or missing state all keep
        // polling (InProgress) instead of reporting done.
        for body in [
            &br#"{"data":{"id":"m1"}}"#[..],
            &br#"{"data":{}}"#[..],
            &br#"{"data":{"processing_info":{}}}"#[..],
            &br#"{"data":{"processing_info":{"state":"weird"}}}"#[..],
        ] {
            assert!(
                matches!(
                    parse_status(body),
                    UploadStatus::InProgress | UploadStatus::Pending
                ),
                "{body:?} must not report success"
            );
        }
    }
}
