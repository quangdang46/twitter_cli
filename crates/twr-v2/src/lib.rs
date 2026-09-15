//! `twr-v2` — official API v2 backend (Phase P4, feature-gated).
//!
//! Cookie backend stays the default; `--backend api-v2` routes covered
//! commands through here (OAuth2 Bearer, no cookie ban-risk). Modules:
//! [`oauth`] (PKCE flow + token store), [`backend`] (routing + v2 URLs),
//! [`video`] (chunked video upload + STATUS polling).

pub mod backend;
pub mod oauth;
pub mod video;

pub use backend::{route, search_url, tweet_url, Backend, ReplyScope, SearchScope, V2_API_ROOT};
pub use oauth::{OAuth2Tokens, DEFAULT_REDIRECT_URI, DEFAULT_SCOPES};
pub use video::{UploadStatus, MAX_VIDEO_BYTES, VIDEO_CHUNK_BYTES};
