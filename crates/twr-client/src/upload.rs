//! Media upload: INIT→APPEND→FINALIZE against
//! `upload.twitter.com/i/media/upload.json` (plan §1.2 Upload + PR #41).
//!
//! Three divergences from the Python original (PR #41 + issue #1, adopted here):
//! - Raw-binary multipart APPEND instead of base64 (`media` bytes field,
//!   ~1/3 less transfer overhead).
//! - Chunked GIF up to 15MB via `media_category=tweet_gif` with 1MB APPEND
//!   chunks (plain images stay ≤5MB, single APPEND).
//! - Video (issue #1) via `media_category=tweet_video` with 5MB APPEND
//!   chunks, then a bounded STATUS poll — video transcodes server-side and
//!   is NOT attachable to a tweet until `processing_info.state` is
//!   `succeeded`.
//!
//! ## Why video uses the same `/i/` endpoint (issue #1 said otherwise)
//!
//! The issue proposed moving video to `upload.twitter.com/1.1/media/upload.json`.
//! That is the **OAuth 1.0a public-API** path. twr authenticates with **web
//! session cookies** (`auth_token` + `ct0`), which is the `/i/` path — the
//! same one images already use, and the one `tweet_gif` proves works with a
//! `media_category` on it. Two further facts confirm `/i/` is correct here:
//! - `/1.1/` requires an OAuth 1.0a signature twr does not and cannot produce
//!   from cookie auth (it fails with error 32 "Could not authenticate you").
//! - X **sunset the v1.1 upload host on 2025-06-09**; it now answers 403 with
//!   an empty body. Switching would have broken working image uploads.
//!
//! The real image/video split is `media_category`, not the URL. Verified
//! 2026-09-30 against X docs, the `large-video-upload` reference client, and
//! third-party web-session uploaders.
//!
//! `--compress N` (image-crate re-encode, never for GIF/video) is a `compress`
//! cargo feature — without it the flag fails loudly at the CLI layer.
//!
//! Pure orchestration over [`crate::HttpTransport`]; the auth headers come
//! from the caller (same `build_headers` set as GraphQL).

use crate::{HttpTransport, TransportError};
use std::time::Duration;

pub const UPLOAD_URL: &str = "https://upload.twitter.com/i/media/upload.json";
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_GIF_BYTES: u64 = 15 * 1024 * 1024;
/// Video cap. X documents 512MB; twr caps lower (matching the api-v2 path's
/// `MAX_VIDEO_BYTES`) as a safety default — configurable, never unbounded.
pub const MAX_VIDEO_BYTES: u64 = 128 * 1024 * 1024;
/// PR #41 chunk size for tweet_gif APPEND segments.
pub const GIF_CHUNK_BYTES: usize = 1024 * 1024;
/// APPEND segment size for video (X's documented per-chunk ceiling is 5MB).
pub const VIDEO_CHUNK_BYTES: usize = 5 * 1024 * 1024;

/// STATUS poll pacing. `check_after_secs` from the server wins when present;
/// these bound the wait between polls and the total give-up point.
pub const STATUS_POLL_FALLBACK_SECS: u64 = 5;
pub const STATUS_POLL_TIMEOUT_SECS: u64 = 5 * 60;

// Size-cap invariants, checked at compile time so a bad edit to the
// constants above cannot ship a cap that rejects valid media or lets an
// oversized file through to the uploader.
const _: () = {
    // A 6MB GIF must clear the pre-flight check and reach the chunked
    // uploader (issue #1 fixed the flat 5MB cap that used to stop it).
    assert!(MAX_GIF_BYTES > MAX_IMAGE_BYTES);
    assert!(MAX_GIF_BYTES == 15 * 1024 * 1024);
    // Video must clear the image cap but stay within X's documented 512MB.
    assert!(MAX_VIDEO_BYTES > MAX_IMAGE_BYTES);
    assert!(MAX_VIDEO_BYTES <= 512 * 1024 * 1024);
};

const IMAGE_TYPES: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];

const VIDEO_TYPES: &[(&str, &str)] = &[
    ("mp4", "video/mp4"),
    ("m4v", "video/mp4"),
    ("mov", "video/quicktime"),
    ("webm", "video/webm"),
];

/// Classify a path by extension. Returns the MIME type or `None`.
/// Covers both image and video extensions — callers that only want images
/// should gate on [`is_video`] first (the CLI does, in `validate_images`).
pub fn mime_for(path: &std::path::Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    IMAGE_TYPES
        .iter()
        .chain(VIDEO_TYPES.iter())
        .find(|(e, _)| *e == ext)
        .map(|(_, m)| *m)
}

/// Whether this file takes the chunked-GIF path (PR #41).
pub fn is_gif(mime: &str) -> bool {
    mime == "image/gif"
}

/// Whether this file takes the async video path (issue #1).
pub fn is_video(mime: &str) -> bool {
    mime.starts_with("video/")
}

/// Max bytes for this MIME (128MB video, 15MB GIF, 5MB plain image).
pub fn max_bytes_for(mime: &str) -> u64 {
    if is_video(mime) {
        MAX_VIDEO_BYTES
    } else if is_gif(mime) {
        MAX_GIF_BYTES
    } else {
        MAX_IMAGE_BYTES
    }
}

/// INIT params: `command/total_bytes/media_type`, plus `media_category` for the
/// async categories. Form-urlencoded body.
///
/// `tweet_gif` and `tweet_video` are what switch the endpoint into async
/// processing mode; omitting them is why a naive widening of the image
/// allowlist "fails later at INIT" (issue #1).
pub fn init_params(total_bytes: u64, mime: &str) -> Vec<(String, String)> {
    let mut p = vec![
        ("command".into(), "INIT".into()),
        ("total_bytes".into(), total_bytes.to_string()),
        ("media_type".into(), mime.into()),
    ];
    if is_gif(mime) {
        p.push(("media_category".into(), "tweet_gif".into()));
    } else if is_video(mime) {
        p.push(("media_category".into(), "tweet_video".into()));
    }
    p
}

/// Split bytes into APPEND segments: 5MB for video, 1MB for GIF, a single
/// segment for plain images.
pub fn chunk_segments<'a>(data: &'a [u8], mime: &str) -> Vec<&'a [u8]> {
    if is_video(mime) {
        return data.chunks(VIDEO_CHUNK_BYTES).collect();
    }
    if !is_gif(mime) {
        return vec![data];
    }
    data.chunks(GIF_CHUNK_BYTES).collect()
}

/// Parse `media_id_string` out of an INIT response body.
pub fn parse_media_id(body: &[u8]) -> Result<String, UploadError> {
    let v: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| UploadError::BadInit("INIT returned invalid JSON".into()))?;
    v.get("media_id_string")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| UploadError::BadInit("INIT did not return media_id".into()))
}

/// Server-side transcoding state for async media (video, and GIF on the
/// paths where X decides to transcode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessingState {
    Pending,
    InProgress,
    Succeeded,
    Failed(String),
}

/// Parse the `processing_info` block out of a FINALIZE or STATUS body.
///
/// Absent `processing_info` means X processed the media synchronously — which
/// is the normal case for images, and must NOT be treated as success for
/// video (issue #1: a naive immediate read of the finalize response is the
/// empty-`media_id_string` race). Callers poll when this returns `None`.
pub fn parse_processing(body: &[u8]) -> Option<ProcessingState> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let info = v.get("processing_info")?;
    let state = info.get("state").and_then(|s| s.as_str())?;
    match state {
        "pending" => Some(ProcessingState::Pending),
        "in_progress" => Some(ProcessingState::InProgress),
        "succeeded" => Some(ProcessingState::Succeeded),
        "failed" => Some(ProcessingState::Failed(
            info.pointer("/error/message")
                .and_then(|s| s.as_str())
                .unwrap_or("video processing failed; re-encode as H.264/AAC MP4 and retry")
                .to_string(),
        )),
        // An unrecognised state is not success — keep polling until timeout.
        _ => Some(ProcessingState::InProgress),
    }
}

/// Seconds the server asks us to wait before the next STATUS poll.
pub fn check_after_secs(body: &[u8]) -> u64 {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/processing_info/check_after_secs")
                .and_then(|s| s.as_u64())
        })
        .filter(|s| *s > 0)
        .unwrap_or(STATUS_POLL_FALLBACK_SECS)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UploadError {
    #[error("media upload: {0}")]
    Io(String),
    #[error("media upload INIT failed: {0}")]
    BadInit(String),
    #[error("media upload APPEND failed (HTTP {0})")]
    BadAppend(u16),
    #[error("media upload FINALIZE failed (HTTP {0})")]
    BadFinalize(u16),
    /// Async media (video) transcoding failed server-side. Carries X's own
    /// message where one was given, so the user sees the real reason.
    #[error("media processing failed: {0}")]
    ProcessingFailed(String),
    /// Async media never reached `succeeded` inside the poll budget.
    #[error(
        "media still processing after {secs}s; X is still transcoding. \
         The upload is not lost — re-run the post later, or check the draft. \
         Raise TWR_UPLOAD_POLL_TIMEOUT_SECS to wait longer."
    )]
    ProcessingTimeout { secs: u64 },
}

/// Overall wait budget for async processing, overridable for slow networks.
pub fn poll_timeout() -> Duration {
    Duration::from_secs(
        std::env::var("TWR_UPLOAD_POLL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(STATUS_POLL_TIMEOUT_SECS),
    )
}

/// Poll `command=STATUS` on the upload endpoint until processing succeeds.
///
/// Video FINALIZE returns while X is still transcoding; attaching the media
/// id to a tweet before `succeeded` fails server-side. This loop is the fix
/// for that race — it honours the server's `check_after_secs` and gives up
/// with an actionable error rather than a generic upload failure.
pub async fn await_processing(
    transport: &dyn HttpTransport,
    base_headers: &[(&str, &str)],
    media_id: &str,
) -> Result<(), UploadError> {
    // `tokio::time::Instant` (not `std::time::Instant`): it is the clock the
    // `sleep` below actually observes, so the deadline and the sleeps agree —
    // and `#[tokio::test(start_paused)]` can exercise the timeout in virtual
    // time instead of waiting out the real 5 minutes.
    let deadline = tokio::time::Instant::now() + poll_timeout();
    loop {
        let url = format!("{UPLOAD_URL}?command=STATUS&media_id={media_id}");
        let resp = transport
            .get(&url, base_headers)
            .await
            .map_err(|e| UploadError::Io(e.to_string()))?;
        if resp.status >= 400 {
            return Err(UploadError::BadFinalize(resp.status));
        }
        match parse_processing(&resp.body) {
            // No processing_info on a STATUS response means the media is done.
            Some(ProcessingState::Succeeded) | None => return Ok(()),
            Some(ProcessingState::Failed(reason)) => {
                return Err(UploadError::ProcessingFailed(reason))
            }
            Some(ProcessingState::Pending) | Some(ProcessingState::InProgress) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(UploadError::ProcessingTimeout {
                secs: poll_timeout().as_secs(),
            });
        }
        tokio::time::sleep(Duration::from_secs(check_after_secs(&resp.body))).await;
    }
}

/// Full upload: INIT → APPEND segment(s) → FINALIZE. Returns the media id.
/// `headers` are the caller's auth header set; per-step Content-Type
/// overrides are applied internally (form-urlencoded for INIT/FINALIZE,
/// raw-binary multipart for APPEND).
pub async fn upload_media(
    transport: &dyn HttpTransport,
    base_headers: &[(&str, &str)],
    data: Vec<u8>,
    mime: &str,
) -> Result<String, UploadError> {
    // ── INIT ──
    let mut init_headers: Vec<(&str, &str)> = base_headers.to_vec();
    init_headers.push(("Content-Type", "application/x-www-form-urlencoded"));
    let init_body = init_params(data.len() as u64, mime)
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let resp = transport
        .post_json(UPLOAD_URL, &init_headers, init_body.as_bytes())
        .await
        .map_err(|e| UploadError::Io(e.to_string()))?;
    if resp.status >= 400 {
        return Err(UploadError::BadInit(format!("HTTP {}", resp.status)));
    }
    let media_id = parse_media_id(&resp.body)?;

    // ── APPEND (raw-binary multipart, PR #41) ──
    for (i, segment) in chunk_segments(&data, mime).iter().enumerate() {
        let boundary = format!("----twr-upload-{media_id}-{i}");
        let mut parts: Vec<u8> = Vec::new();
        let field = |name: &str, value: &str| {
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
        };
        parts.extend_from_slice(field("command", "APPEND").as_bytes());
        parts.extend_from_slice(field("media_id", &media_id).as_bytes());
        parts.extend_from_slice(field("segment_index", &i.to_string()).as_bytes());
        parts.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"media\"; filename=\"media\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
        );
        parts.extend_from_slice(segment);
        parts.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        // Rebuild header refs with the multipart content type (borrow-safe:
        // collect owned Strings first, then refs).
        let ct = format!("multipart/form-data; boundary={boundary}");
        let mut owned: Vec<(String, String)> = base_headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        owned.push(("Content-Type".into(), ct));
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let resp = transport
            .post_json(UPLOAD_URL, &refs, &parts)
            .await
            .map_err(|e| UploadError::Io(e.to_string()))?;
        if resp.status >= 400 {
            return Err(UploadError::BadAppend(resp.status));
        }
    }

    // ── FINALIZE ──
    let fin_body = format!("command=FINALIZE&media_id={media_id}");
    let resp = transport
        .post_json(UPLOAD_URL, &init_headers, fin_body.as_bytes())
        .await
        .map_err(|e| UploadError::Io(e.to_string()))?;
    if resp.status >= 400 {
        return Err(UploadError::BadFinalize(resp.status));
    }

    // ── Poll async media (video) until X finishes transcoding ──
    // Images and GIFs finalize synchronously; video returns a processing
    // state and is not attachable to a tweet until it is `succeeded`.
    if is_video(mime) {
        // Honor a server that already reported success inline on FINALIZE.
        if parse_processing(&resp.body) != Some(ProcessingState::Succeeded) {
            await_processing(transport, base_headers, &media_id).await?;
        }
    }

    Ok(media_id)
}

impl From<TransportError> for UploadError {
    fn from(e: TransportError) -> Self {
        UploadError::Io(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_and_limits() {
        let gif = std::path::Path::new("a.gif");
        assert_eq!(mime_for(gif), Some("image/gif"));
        assert!(is_gif("image/gif"));
        assert_eq!(max_bytes_for("image/gif"), MAX_GIF_BYTES);
        assert_eq!(max_bytes_for("image/png"), MAX_IMAGE_BYTES);
        assert_eq!(mime_for(std::path::Path::new("a.bmp")), None);
    }

    /// Issue #1: video rides the same `/i/` endpoint, gated by its own size
    /// cap and `media_category` — not by a different URL.
    #[test]
    fn video_mime_limits_and_category() {
        assert_eq!(mime_for(std::path::Path::new("a.mp4")), Some("video/mp4"));
        assert_eq!(
            mime_for(std::path::Path::new("a.MOV")),
            Some("video/quicktime")
        );
        assert_eq!(mime_for(std::path::Path::new("a.webm")), Some("video/webm"));
        assert!(is_video("video/mp4"));
        assert!(!is_video("image/png"));
        // The video/image cap relationship is a compile-time invariant
        // (enforced next to the constants), so assert the per-MIME routing
        // that actually depends on it here.
        assert_eq!(max_bytes_for("video/mp4"), MAX_VIDEO_BYTES);
        assert_eq!(max_bytes_for("image/png"), MAX_IMAGE_BYTES);
        let init = init_params(100, "video/mp4");
        assert!(init.contains(&("media_category".into(), "tweet_video".into())));
        assert!(init.contains(&("media_type".into(), "video/mp4".into())));
    }

    #[test]
    fn init_params_adds_tweet_gif_only_for_gif() {
        let gif = init_params(100, "image/gif");
        assert!(gif.contains(&("media_category".into(), "tweet_gif".into())));
        let png = init_params(100, "image/png");
        assert!(!png.iter().any(|(k, _)| k == "media_category"));
    }

    /// Video APPENDs in 5MB segments; segment lengths must reconstruct the
    /// file exactly (a gap fails the whole upload at FINALIZE, not at the
    /// chunk that caused it).
    #[test]
    fn video_chunks_are_five_mb_and_contiguous() {
        assert_eq!(chunk_segments(&[0u8; 10], "video/mp4").len(), 1);
        let big = vec![0u8; 2 * VIDEO_CHUNK_BYTES + 7];
        let segs = chunk_segments(&big, "video/mp4");
        assert_eq!(segs.len(), 3);
        assert!(segs.iter().all(|s| s.len() <= VIDEO_CHUNK_BYTES));
        assert_eq!(segs.iter().map(|s| s.len()).sum::<usize>(), big.len());
    }

    #[test]
    fn processing_states_parse() {
        assert_eq!(
            parse_processing(br#"{"processing_info":{"state":"succeeded"}}"#),
            Some(ProcessingState::Succeeded)
        );
        assert_eq!(
            parse_processing(br#"{"processing_info":{"state":"in_progress"}}"#),
            Some(ProcessingState::InProgress)
        );
        assert_eq!(
            parse_processing(br#"{"processing_info":{"state":"pending"}}"#),
            Some(ProcessingState::Pending)
        );
        assert!(matches!(
            parse_processing(br#"{"processing_info":{"state":"failed"}}"#),
            Some(ProcessingState::Failed(_))
        ));
        // Images finalize synchronously: no processing_info block at all.
        assert_eq!(parse_processing(br#"{"media_id_string":"1"}"#), None);
        // An unknown state is never silently treated as success.
        assert_eq!(
            parse_processing(br#"{"processing_info":{"state":"weird"}}"#),
            Some(ProcessingState::InProgress)
        );
    }

    #[test]
    fn failed_state_carries_server_message() {
        let Some(ProcessingState::Failed(reason)) = parse_processing(
            br#"{"processing_info":{"state":"failed","error":{"message":"unsupported codec"}}}"#,
        ) else {
            panic!("expected failed state");
        };
        assert_eq!(reason, "unsupported codec");
    }

    #[test]
    fn check_after_secs_honors_server_with_fallback() {
        assert_eq!(
            check_after_secs(br#"{"processing_info":{"check_after_secs":7}}"#),
            7
        );
        assert_eq!(check_after_secs(br#"{}"#), STATUS_POLL_FALLBACK_SECS);
        // A zero hint must not turn the poll into a hot loop.
        assert_eq!(
            check_after_secs(br#"{"processing_info":{"check_after_secs":0}}"#),
            STATUS_POLL_FALLBACK_SECS
        );
    }

    #[test]
    fn chunks_single_for_images_many_for_big_gif() {
        assert_eq!(chunk_segments(&[0u8; 10], "image/png").len(), 1);
        let big = vec![0u8; 2 * GIF_CHUNK_BYTES + 10];
        assert_eq!(chunk_segments(&big, "image/gif").len(), 3);
    }

    #[test]
    fn parse_media_id_ok_and_errors() {
        assert_eq!(
            parse_media_id(br#"{"media_id_string":"123"}"#).unwrap(),
            "123"
        );
        assert!(parse_media_id(br"nope").is_err());
        assert!(parse_media_id(br#"{}"#).is_err());
    }

    /// The happy-path AC: video FINALIZE returns a processing state, and the
    /// upload polls STATUS until the server says `succeeded` — so the caller
    /// never receives a media id that isn't attachable yet.
    #[tokio::test]
    async fn video_upload_polls_until_succeeded() {
        struct Polling(std::sync::Mutex<u8>);
        #[async_trait::async_trait]
        impl HttpTransport for Polling {
            async fn get(
                &self,
                url: &str,
                _h: &[(&str, &str)],
            ) -> Result<crate::TransportResponse, TransportError> {
                assert!(url.contains("command=STATUS"));
                assert!(url.contains("media_id=m1"));
                let mut n = self.0.lock().unwrap();
                *n += 1;
                Ok(crate::TransportResponse {
                    status: 200,
                    // Still transcoding twice, then done.
                    body: if *n < 3 {
                        br#"{"processing_info":{"state":"in_progress"}}"#.to_vec()
                    } else {
                        br#"{"processing_info":{"state":"succeeded"}}"#.to_vec()
                    },
                })
            }
            async fn post_json(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
                body: &[u8],
            ) -> Result<crate::TransportResponse, TransportError> {
                let b = String::from_utf8_lossy(body);
                if b.contains("command=INIT") {
                    Ok(crate::TransportResponse {
                        status: 200,
                        body: br#"{"media_id_string":"m1"}"#.to_vec(),
                    })
                } else if b.contains("command=FINALIZE") {
                    Ok(crate::TransportResponse {
                        status: 200,
                        body: br#"{"processing_info":{"state":"pending"}}"#.to_vec(),
                    })
                } else {
                    Ok(crate::TransportResponse {
                        status: 204,
                        body: vec![],
                    })
                }
            }
        }
        let t = Polling(std::sync::Mutex::new(0));
        let id = upload_media(&t, &[], vec![0u8; 16], "video/mp4")
            .await
            .unwrap();
        assert_eq!(id, "m1");
        assert!(*t.0.lock().unwrap() >= 3, "must poll until succeeded");
    }

    /// AC: a processing failure surfaces X's reason, not a generic failure.
    #[tokio::test]
    async fn video_processing_failure_surfaces_reason() {
        struct FailProc;
        #[async_trait::async_trait]
        impl HttpTransport for FailProc {
            async fn get(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
            ) -> Result<crate::TransportResponse, TransportError> {
                Ok(crate::TransportResponse {
                    status: 200,
                    body:
                        br#"{"processing_info":{"state":"failed","error":{"message":"bad codec"}}}"#
                            .to_vec(),
                })
            }
            async fn post_json(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
                body: &[u8],
            ) -> Result<crate::TransportResponse, TransportError> {
                let b = String::from_utf8_lossy(body);
                let body = if b.contains("command=INIT") {
                    br#"{"media_id_string":"m1"}"#.to_vec()
                } else if b.contains("command=FINALIZE") {
                    br#"{"processing_info":{"state":"pending"}}"#.to_vec()
                } else {
                    vec![]
                };
                Ok(crate::TransportResponse { status: 200, body })
            }
        }
        let err = upload_media(&FailProc, &[], vec![0u8; 8], "video/mp4")
            .await
            .unwrap_err();
        assert_eq!(err, UploadError::ProcessingFailed("bad codec".into()));
    }

    /// AC: the timeout is an actionable error naming the knob, not a bare
    /// upload failure. Uses `tokio::time::pause` so it costs no wall time.
    #[tokio::test(start_paused = true)]
    async fn video_processing_timeout_is_actionable() {
        struct NeverReady;
        #[async_trait::async_trait]
        impl HttpTransport for NeverReady {
            async fn get(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
            ) -> Result<crate::TransportResponse, TransportError> {
                Ok(crate::TransportResponse {
                    status: 200,
                    body: br#"{"processing_info":{"state":"in_progress","check_after_secs":30}}"#
                        .to_vec(),
                })
            }
            async fn post_json(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
                body: &[u8],
            ) -> Result<crate::TransportResponse, TransportError> {
                let b = String::from_utf8_lossy(body);
                let body = if b.contains("command=INIT") {
                    br#"{"media_id_string":"m1"}"#.to_vec()
                } else {
                    b"{}".to_vec()
                };
                Ok(crate::TransportResponse { status: 200, body })
            }
        }
        let err = upload_media(&NeverReady, &[], vec![0u8; 8], "video/mp4")
            .await
            .unwrap_err();
        let UploadError::ProcessingTimeout { secs } = err else {
            panic!("expected timeout, got {err:?}");
        };
        assert_eq!(secs, STATUS_POLL_TIMEOUT_SECS);
        // The message must point at the override, not just say "failed".
        let msg = UploadError::ProcessingTimeout { secs }.to_string();
        assert!(msg.contains("TWR_UPLOAD_POLL_TIMEOUT_SECS"), "{msg}");
    }

    /// Image uploads must NOT start polling — they finalize synchronously,
    /// and a spurious GET would be a wasted round trip per image.
    #[tokio::test]
    async fn image_upload_does_not_poll() {
        struct NoPoll(std::sync::atomic::AtomicBool);
        #[async_trait::async_trait]
        impl HttpTransport for NoPoll {
            async fn get(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
            ) -> Result<crate::TransportResponse, TransportError> {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(crate::TransportResponse {
                    status: 200,
                    body: vec![],
                })
            }
            async fn post_json(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
                body: &[u8],
            ) -> Result<crate::TransportResponse, TransportError> {
                let b = String::from_utf8_lossy(body);
                let body = if b.contains("command=INIT") {
                    br#"{"media_id_string":"m1"}"#.to_vec()
                } else {
                    b"{}".to_vec()
                };
                Ok(crate::TransportResponse { status: 200, body })
            }
        }
        let t = NoPoll(std::sync::atomic::AtomicBool::new(false));
        upload_media(&t, &[], b"D".to_vec(), "image/png")
            .await
            .unwrap();
        assert!(
            !t.0.load(std::sync::atomic::Ordering::SeqCst),
            "image path must not poll STATUS"
        );
    }

    /// End-to-end over a fake transport: INIT→APPEND→FINALIZE with raw-binary
    /// multipart on the APPEND leg.
    #[tokio::test]
    async fn upload_flow_succeeds_over_fake_transport() {
        struct Fake;
        #[async_trait::async_trait]
        impl HttpTransport for Fake {
            async fn get(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
            ) -> Result<crate::TransportResponse, TransportError> {
                unreachable!()
            }
            async fn post_json(
                &self,
                _u: &str,
                headers: &[(&str, &str)],
                body: &[u8],
            ) -> Result<crate::TransportResponse, TransportError> {
                let body_s = String::from_utf8_lossy(body);
                if body_s.contains("command=INIT") {
                    assert!(headers
                        .iter()
                        .any(|(k, v)| *k == "Content-Type" && v.contains("urlencoded")));
                    Ok(crate::TransportResponse {
                        status: 200,
                        body: br#"{"media_id_string":"m1"}"#.to_vec(),
                    })
                } else if body_s.contains("command=FINALIZE") {
                    Ok(crate::TransportResponse {
                        status: 200,
                        body: b"{}".to_vec(),
                    })
                } else {
                    // APPEND leg: raw-binary multipart, not base64.
                    assert!(headers
                        .iter()
                        .any(|(k, v)| *k == "Content-Type" && v.contains("multipart/form-data")));
                    assert!(body.windows(4).any(|w| w == b"\x89PNG" || w == b"DATA"));
                    Ok(crate::TransportResponse {
                        status: 204,
                        body: vec![],
                    })
                }
            }
        }
        let id = upload_media(&Fake, &[], b"DATA-DATA".to_vec(), "image/png")
            .await
            .unwrap();
        assert_eq!(id, "m1");
    }

    #[tokio::test]
    async fn upload_surfaces_append_failure() {
        struct FailAppend;
        #[async_trait::async_trait]
        impl HttpTransport for FailAppend {
            async fn get(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
            ) -> Result<crate::TransportResponse, TransportError> {
                unreachable!()
            }
            async fn post_json(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
                body: &[u8],
            ) -> Result<crate::TransportResponse, TransportError> {
                if String::from_utf8_lossy(body).contains("command=INIT") {
                    Ok(crate::TransportResponse {
                        status: 200,
                        body: br#"{"media_id_string":"m1"}"#.to_vec(),
                    })
                } else {
                    Ok(crate::TransportResponse {
                        status: 500,
                        body: vec![],
                    })
                }
            }
        }
        assert_eq!(
            upload_media(&FailAppend, &[], b"D".to_vec(), "image/png")
                .await
                .unwrap_err(),
            UploadError::BadAppend(500)
        );
    }
}
