//! Default [`crate::HttpTransport`] implementation: `wreq` with browser-matched
//! TLS/HTTP2 fingerprinting ("browser-compatible transport fingerprinting",
//! COMPREHENSIVEPLANFORTWITTERCLI.md §3 — plain `reqwest` is never used
//! because its TLS handshake gives it away as non-browser traffic).
//!
//! This is the P0-1 spike deliverable (bead `twitter_cli-5o3.2.1`): prove
//! wreq's impersonation actually produces a browser-shaped TLS/HTTP2
//! fingerprint. The spike test in this module hits `tls.peet.ws` — a public,
//! neutral fingerprint-inspection service, NOT x.com — deliberately, so this
//! crate's test suite never touches a live X/Twitter account. Making an
//! authenticated request to a real account is P0-4's job and requires a
//! separate, explicitly confirmed step with real credentials.

use crate::{HttpTransport, TransportError, TransportResponse};
use async_trait::async_trait;
use wreq::Client;
use wreq_util::Emulation;

/// Wraps a `wreq::Client` pre-configured to impersonate a recent Chrome
/// release. One instance should be reused across requests (wreq's client
/// holds the connection pool + cookie store).
pub struct WreqTransport {
    client: Client,
}

/// Resolve the TLS/HTTP2 emulation profile named by `TWITTER_EMULATION`
/// (issue #6). Accepts the serde names of [`Emulation`] variants
/// (`"chrome_131"`, `"chrome_133"`, `"firefox_135"`, `"safari_18"`, …).
/// Unset or unrecognized values keep the Chrome131 default.
pub fn emulation_from_env() -> Emulation {
    match std::env::var("TWITTER_EMULATION") {
        Ok(name) if !name.trim().is_empty() => {
            serde_json::from_value(serde_json::Value::String(name.trim().to_string()))
                .unwrap_or(Emulation::Chrome131)
        }
        _ => Emulation::Chrome131,
    }
}

impl WreqTransport {
    /// Build a transport impersonating a recent Chrome release.
    ///
    /// The profile defaults to `Emulation::Chrome131` and can be overridden
    /// with `TWITTER_EMULATION` (e.g. `chrome_133`) — see
    /// [`emulation_from_env`]. Note: which exact `Emulation::ChromeNNN`
    /// variant exists depends on the `wreq-util` version this workspace
    /// resolves to (older Rust toolchains here pin an older wreq/wreq-util
    /// pair than the crates' latest docs show) — if this fails to compile
    /// after a dependency bump, check `wreq_util::Emulation`'s available
    /// variants and adjust.
    pub fn new_chrome() -> Result<Self, TransportError> {
        Self::with_emulation(emulation_from_env())
    }

    /// Build a transport with an explicit emulation profile.
    pub fn with_emulation(emulation: Emulation) -> Result<Self, TransportError> {
        Self::build(emulation, None)
    }

    /// Chrome-emulating transport with an explicit total request timeout —
    /// for short-lived utility calls (the GitHub update check/download,
    /// issue #25) that must never stall a command.
    pub fn new_chrome_with_timeout(timeout: std::time::Duration) -> Result<Self, TransportError> {
        Self::build(emulation_from_env(), Some(timeout))
    }

    fn build(
        emulation: Emulation,
        timeout: Option<std::time::Duration>,
    ) -> Result<Self, TransportError> {
        // cookie_store(false): twr sends a full explicit `Cookie:` header
        // on every request (Method C paste, code-226 gate). Letting wreq
        // ALSO jar cookies means Set-Cookie responses (e.g. rotated guest
        // tokens, cf_clearance refreshes) silently append/override the
        // explicit header on later requests in the same process — a
        // stateful mutation of auth material the caller cannot see. The
        // caller owns cookies; the transport must be stateless.
        // (Live-found 2026-09-20: CreateList 214'd through this transport
        // while succeeding via curl with byte-identical vars/features —
        // cookie-jar interference is the prime suspect.)
        let mut builder = Client::builder().emulation(emulation).cookie_store(false);
        if let Some(t) = timeout {
            builder = builder.timeout(t);
        }
        let client = builder
            .build()
            .map_err(|e| TransportError::Io(format!("failed to build wreq client: {e}")))?;
        Ok(Self { client })
    }

    /// Construct from an already-built `wreq::Client`, for callers that need
    /// custom emulation/proxy/timeout settings beyond [`Self::new_chrome`].
    pub fn from_client(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HttpTransport for WreqTransport {
    async fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<TransportResponse, TransportError> {
        let mut req = self.client.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?
            .to_vec();
        Ok(TransportResponse { status, body })
    }

    async fn post_json(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<TransportResponse, TransportError> {
        let mut req = self.client.post(url).body(body.to_vec());
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?
            .to_vec();
        Ok(TransportResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P0-1 spike: does wreq's Chrome impersonation produce a browser-shaped
    /// TLS/HTTP2 fingerprint? `tls.peet.ws/api/all` is a public fingerprint
    /// inspector (not X/Twitter) built exactly for this kind of check.
    ///
    /// `#[ignore]`d by default because it needs network access; the P0 spike
    /// itself was run manually (see the bead's close notes for the observed
    /// result) rather than relying on this running in CI, since CI runners'
    /// egress can itself look automated regardless of TLS fingerprint.
    #[tokio::test]
    #[ignore = "hits a live network service; run manually with --ignored for the P0 spike"]
    async fn chrome_emulation_reaches_a_neutral_fingerprint_checker() {
        let transport = WreqTransport::new_chrome().expect("client should build");
        let resp = transport
            .get("https://tls.peet.ws/api/all", &[])
            .await
            .expect("request should succeed");
        assert_eq!(resp.status, 200);
        let body = String::from_utf8_lossy(&resp.body);
        // tls.peet.ws echoes back a "user_agent" field derived from the
        // negotiated fingerprint's matching browser profile.
        assert!(
            body.to_lowercase().contains("chrome"),
            "expected a Chrome-shaped fingerprint, got: {body}"
        );
    }

    #[test]
    fn new_chrome_builds_without_error() {
        // No network — just confirms the emulation profile + feature flags
        // are wired correctly and the client constructs.
        WreqTransport::new_chrome().expect("client should build offline");
    }

    #[test]
    fn emulation_from_env_defaults_and_overrides() {
        // Env-dependent: only assert the default when the var is unset.
        if std::env::var("TWITTER_EMULATION").is_err() {
            assert_eq!(emulation_from_env(), Emulation::Chrome131);
        }
        // A serde name round-trips through the same path the env uses.
        let parsed: Emulation =
            serde_json::from_value(serde_json::Value::String("chrome_133".into())).unwrap();
        assert_eq!(parsed, Emulation::Chrome133);
        // Garbage falls back rather than panicking.
        let bad: Result<Emulation, _> =
            serde_json::from_value(serde_json::Value::String("not_a_browser".into()));
        assert!(bad.is_err());
    }
}
