//! `twr-client` — transport-abstracted client for X's internal GraphQL API.
//!
//! Per `PLAN.md` §3/§7, all HTTP goes through the [`HttpTransport`] trait so
//! the default `wreq`-based impersonation transport can be swapped for a
//! `curl-impersonate` subprocess fallback without touching call sites.
//!
//! Modules: [`headers`] (full `_build_headers` port), [`throttle`]
//! (per-endpoint token bucket + page-count/jitter math), [`timeline`] (the
//! `_fetch_timeline` pagination loop), [`wreq_transport`] (default impl).

pub mod guest;
pub mod headers;
pub mod throttle;
pub mod timeline;
pub mod upload;
mod wreq_transport;
pub use wreq_transport::WreqTransport;

pub use headers::{
    build_headers, notification_params, notifications_url, Credentials, HeaderInput, Os,
    BEARER_TOKEN,
};
pub use throttle::{jittered_delay_secs, page_count, use_post, BucketConfig, Throttle, POST_OPS};
pub use timeline::{backoff_delays_secs, fetch_timeline, Page, PageError, TimelineResult};

use async_trait::async_trait;

/// `TWITTER_PROXY` env var (plan §1.2). Read by the transport constructor in
/// the binary crate; documented here so both layers agree on the name.
pub const PROXY_ENV_VAR: &str = "TWITTER_PROXY";

/// Read the proxy URL from the environment, if set and non-empty.
pub fn proxy_from_env() -> Option<String> {
    std::env::var(PROXY_ENV_VAR).ok().filter(|v| !v.is_empty())
}

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<TransportResponse, TransportError>;
    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<TransportResponse, TransportError>;
}

pub struct TransportResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("transport io error: {0}")]
    Io(String),
    #[error("transport timeout")]
    Timeout,
}
