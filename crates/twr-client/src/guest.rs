//! Guest-tier degraded-mode reads (bead twitter_cli-5o3.7.4).
//!
//! x-cli-go 3-tier model port: when full session auth isn't available,
//! guest tokens minted via `api.x.com/1.1/guest/activate.json` cover a
//! narrow op set (profile by handle/id, tweet reads). A `--tier` flag
//! caps/forces the tier used.
//!
//! Pure shapes + token-response parsing here; the HTTP call goes through
//! [`crate::HttpTransport`] via [`activate_guest`] (needs a sender for the
//! POST — kept transport-generic).

/// Access tier, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tier {
    /// No auth at all (only fully public syndication shapes — not implemented).
    Syndication,
    /// Guest token (this module).
    Guest,
    /// Full session (default everywhere).
    #[default]
    Session,
}

impl Tier {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "syndication" => Some(Tier::Syndication),
            "guest" => Some(Tier::Guest),
            "session" => Some(Tier::Session),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Syndication => "syndication",
            Tier::Guest => "guest",
            Tier::Session => "session",
        }
    }
}

/// Ops the guest tier covers. Everything else requires session.
pub const GUEST_OPS: &[&str] = &["UserByScreenName", "TweetDetail", "UserTweets"];

pub fn guest_covers(operation: &str) -> bool {
    GUEST_OPS.contains(&operation)
}

/// Guest activation endpoint.
pub const GUEST_ACTIVATE_URL: &str = "https://api.x.com/1.1/guest/activate.json";

/// Parse `{"guest_token": "…"}` out of the activation response.
pub fn parse_guest_token(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()?
        .get("guest_token")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Mint a guest token over any transport. POST with empty body.
pub async fn activate_guest(transport: &dyn crate::HttpTransport) -> Result<String, String> {
    let resp = transport
        .post_json(GUEST_ACTIVATE_URL, &[], b"")
        .await
        .map_err(|e| e.to_string())?;
    if !(200..300).contains(&resp.status) {
        return Err(format!("guest activate HTTP {}", resp.status));
    }
    parse_guest_token(&resp.body).ok_or_else(|| "guest activate: no guest_token".to_string())
}

/// Resolve the effective tier: `--tier` caps the maximum used. `None` flag =
/// best available (session when authed, else guest attempt by the caller).
pub fn effective_tier(flag: Option<Tier>, authed: bool) -> Tier {
    match (flag, authed) {
        (Some(Tier::Syndication), _) => Tier::Syndication,
        (Some(Tier::Guest), _) => Tier::Guest,
        (Some(Tier::Session), _) => Tier::Session,
        (None, true) => Tier::Session,
        (None, false) => Tier::Guest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_covers_narrow_op_set() {
        assert!(guest_covers("UserByScreenName"));
        assert!(guest_covers("TweetDetail"));
        assert!(!guest_covers("SearchTimeline"));
        assert!(!guest_covers("CreateTweet"));
    }

    #[test]
    fn token_parses_and_rejects_garbage() {
        assert_eq!(
            parse_guest_token(br#"{"guest_token":"abc"}"#),
            Some("abc".into())
        );
        assert_eq!(parse_guest_token(br"{}"), None);
        assert_eq!(parse_guest_token(b"junk"), None);
    }

    #[test]
    fn tier_flag_caps() {
        assert_eq!(effective_tier(Some(Tier::Guest), true), Tier::Guest);
        assert_eq!(effective_tier(None, true), Tier::Session);
        assert_eq!(effective_tier(None, false), Tier::Guest);
        assert_eq!(Tier::parse("SESSION"), Some(Tier::Session));
    }

    #[tokio::test]
    async fn activate_parses_token_over_fake_transport() {
        struct Fake;
        #[async_trait::async_trait]
        impl crate::HttpTransport for Fake {
            async fn get(
                &self,
                _u: &str,
                _h: &[(&str, &str)],
            ) -> Result<crate::TransportResponse, crate::TransportError> {
                unreachable!()
            }
            async fn post_json(
                &self,
                url: &str,
                _h: &[(&str, &str)],
                _b: &[u8],
            ) -> Result<crate::TransportResponse, crate::TransportError> {
                assert_eq!(url, GUEST_ACTIVATE_URL);
                Ok(crate::TransportResponse {
                    status: 200,
                    body: br#"{"guest_token":"g1"}"#.to_vec(),
                })
            }
        }
        assert_eq!(activate_guest(&Fake).await.unwrap(), "g1");
    }
}
