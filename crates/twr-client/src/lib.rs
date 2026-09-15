//! `twr-client` — transport-abstracted client for X's internal GraphQL API.
//!
//! Per `PLAN.md` §3/§7, all HTTP goes through the [`HttpTransport`] trait so
//! the default `wreq`-based impersonation transport can be swapped for a
//! `curl-impersonate` subprocess fallback without touching call sites.
//!
//! Everything here is a stub — the P0 spike (PLAN.md §9) decides whether this
//! crate proceeds past a single `UserByScreenName` proof of concept.

use async_trait::async_trait;

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<TransportResponse, TransportError>;
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
