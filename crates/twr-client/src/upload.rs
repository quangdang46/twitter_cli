//! Media upload: INIT→APPEND→FINALIZE against
//! `upload.twitter.com/i/media/upload.json` (plan §1.2 Upload + PR #41).
//!
//! Two divergences from the Python original (both from PR #41, adopted here):
//! - Raw-binary multipart APPEND instead of base64 (`media` bytes field,
//!   ~1/3 less transfer overhead).
//! - Chunked GIF up to 15MB via `media_category=tweet_gif` with 1MB APPEND
//!   chunks (plain images stay ≤5MB, single APPEND).
//!
//! `--compress N` (image-crate re-encode, never for GIFs) is a `compress`
//! cargo feature — without it the flag fails loudly at the CLI layer.
//!
//! Pure orchestration over [`crate::HttpTransport`]; the auth headers come
//! from the caller (same `_build_headers` set as GraphQL).

use crate::{HttpTransport, TransportError};

pub const UPLOAD_URL: &str = "https://upload.twitter.com/i/media/upload.json";
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_GIF_BYTES: u64 = 15 * 1024 * 1024;
/// PR #41 chunk size for tweet_gif APPEND segments.
pub const GIF_CHUNK_BYTES: usize = 1024 * 1024;

const IMAGE_TYPES: &[(&str, &str)] = &[
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("png", "image/png"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];

/// Classify a path by extension. Returns the MIME type or `None`.
pub fn mime_for(path: &std::path::Path) -> Option<&'static str> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    IMAGE_TYPES.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

/// Whether this file takes the chunked-GIF path (PR #41).
pub fn is_gif(mime: &str) -> bool {
    mime == "image/gif"
}

/// Max bytes for this MIME (15MB GIF, 5MB everything else).
pub fn max_bytes_for(mime: &str) -> u64 {
    if is_gif(mime) {
        MAX_GIF_BYTES
    } else {
        MAX_IMAGE_BYTES
    }
}

/// INIT params: `command/total_bytes/media_type` (+ `media_category=tweet_gif`
/// for the chunked path). Form-urlencoded body.
pub fn init_params(total_bytes: u64, mime: &str) -> Vec<(String, String)> {
    let mut p = vec![
        ("command".into(), "INIT".into()),
        ("total_bytes".into(), total_bytes.to_string()),
        ("media_type".into(), mime.into()),
    ];
    if is_gif(mime) {
        p.push(("media_category".into(), "tweet_gif".into()));
    }
    p
}

/// Split bytes into 1MB APPEND segments (single segment for non-GIF).
pub fn chunk_segments<'a>(data: &'a [u8], mime: &str) -> Vec<&'a [u8]> {
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

    #[test]
    fn init_params_adds_tweet_gif_only_for_gif() {
        let gif = init_params(100, "image/gif");
        assert!(gif.contains(&("media_category".into(), "tweet_gif".into())));
        let png = init_params(100, "image/png");
        assert!(!png.iter().any(|(k, _)| k == "media_category"));
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
