//! OAuth2 user-context flow for X official API v2 (bead twitter_cli-5o3.6.1).
//!
//! Per PR #31 + issue #21. Standard OAuth 2.0 Authorization Code Flow with
//! PKCE (RFC 7636), the flow X requires for user-context v2 access.
//! Tokens store separately from the cookie-backend session
//! (`~/.twr/oauth2.json`, owner-only) — official API carries no cookie
//! ban-risk, so the two credentials must never mix (plan §11).
//!
//! This module is pure shapes + URL building + token-response parsing.
//! The browser-redirect + local-callback HTTP listener lives in the CLI
//! (`twr login --api-v2`), which is interactive by nature.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// X OAuth2 endpoints.
pub const AUTHORIZE_URL: &str = "https://twitter.com/i/oauth2/authorize";
pub const TOKEN_URL: &str = "https://api.twitter.com/2/oauth2/token";

/// Default local callback (CLI spins a one-shot listener here).
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:8080/callback";

/// Scopes twr requests for v2 user-context use.
pub const DEFAULT_SCOPES: &[&str] = &["tweet.read", "tweet.write", "users.read", "offline.access"];

/// PKCE verifier (43–128 chars from the unreserved set).
pub fn new_verifier() -> String {
    let mut buf = [0u8; 48];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// S256 challenge for a verifier.
pub fn challenge_s256(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// CSRF state token.
pub fn new_state() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Build the authorize URL the user opens in a browser.
pub fn authorize_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    state: &str,
    challenge: &str,
) -> String {
    let scope = scopes.join("%20");
    format!(
        "{AUTHORIZE_URL}?response_type=code&client_id={client_id}&redirect_uri={redirect_uri}&scope={scope}&state={state}&code_challenge={challenge}&code_challenge_method=S256"
    )
}

/// Stored OAuth2 token pair (JSON shape of `~/.twr/oauth2.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuth2Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    /// Unix seconds when `access_token` expires (0 = unknown).
    pub expires_at_secs: u64,
    pub scope: String,
    /// Public client ID used at login — required again for the
    /// `refresh_token` grant (bead p3c). Absent in pre-p3c files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

impl OAuth2Tokens {
    pub fn is_expired(&self, now_secs: u64) -> bool {
        self.expires_at_secs != 0 && now_secs >= self.expires_at_secs.saturating_sub(60)
    }
}

/// Parse a token endpoint response body. `now_secs` anchors `expires_in`.
pub fn parse_token_response(
    body: &[u8],
    now_secs: u64,
    scope_fallback: &str,
) -> Result<OAuth2Tokens, String> {
    let v: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| "oauth2 token response is not JSON".to_string())?;
    let access = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "oauth2 token response lacks access_token".to_string())?;
    let expires_in = v.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(0);
    Ok(OAuth2Tokens {
        access_token: access.to_string(),
        refresh_token: v
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .map(str::to_string),
        token_type: v
            .get("token_type")
            .and_then(|t| t.as_str())
            .unwrap_or("Bearer")
            .to_string(),
        expires_at_secs: if expires_in > 0 {
            now_secs + expires_in
        } else {
            0
        },
        scope: v
            .get("scope")
            .and_then(|t| t.as_str())
            .unwrap_or(scope_fallback)
            .to_string(),
        client_id: None,
    })
}

/// Pending PKCE state between `login --api-v2` step 1 (prints the
/// authorize URL) and step 2 (`--code` exchange). Persisted owner-only
/// next to the token file so the verifier never has to round-trip
/// through the user's clipboard (bead p3c).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPkce {
    pub verifier: String,
    pub state: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub created_at_secs: u64,
}

/// Pending-PKCE path: `~/.twr/oauth2-pending.json`.
pub fn pending_path() -> Option<std::path::PathBuf> {
    home_dir().map(|h| h.join(".twr").join("oauth2-pending.json"))
}

fn write_private_json(path: &std::path::Path, raw: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        let mut f = opts.open(path)?;
        f.write_all(raw.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, raw)?;
    }
    Ok(())
}

/// Persist pending PKCE state (owner-only, same contract as save_tokens).
pub fn save_pending(path: &std::path::Path, pending: &PendingPkce) -> std::io::Result<()> {
    let raw = serde_json::to_string(pending)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    write_private_json(path, &raw)
}

/// Load pending PKCE state (None = missing/unparseable/stale >15min).
pub fn load_pending(path: &std::path::Path) -> Option<PendingPkce> {
    let raw = std::fs::read_to_string(path).ok()?;
    let p: PendingPkce = serde_json::from_str(&raw).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // A pending flow older than 15 minutes is abandoned — the state
    // token's CSRF window is long past.
    if now.saturating_sub(p.created_at_secs) > 15 * 60 {
        return None;
    }
    Some(p)
}

/// Remove the pending file after a successful exchange (or abandon).
pub fn clear_pending(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

/// Minimal percent-encoding for `application/x-www-form-urlencoded`
/// bodies (unreserved set kept verbatim, everything else %XX).
pub fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Extract `(code, state)` from a bare code OR a full redirect URL /
/// query string (`…?code=…&state=…`). Returns None when no `code` param
/// (or bare value) is present.
pub fn extract_code(input: &str) -> Option<(String, Option<String>)> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    // Query-string form: find the `code=` parameter anywhere.
    if let Some(pos) = input.find("code=") {
        // Only treat it as a query param when it starts a parameter
        // (beginning, after '?' or '&') — avoids matching "opcode=".
        let is_param = pos == 0 || input[..pos].ends_with('?') || input[..pos].ends_with('&');
        if is_param {
            let rest = &input[pos + 5..];
            let code = rest.split('&').next().unwrap_or("");
            if code.is_empty() {
                return None;
            }
            let state = rest
                .split('&')
                .filter_map(|kv| kv.strip_prefix("state="))
                .next()
                .map(str::to_string);
            return Some((code.to_string(), state));
        }
    }
    // Bare code: single token, no whitespace.
    if !input.contains(char::is_whitespace) && !input.contains("://") {
        return Some((input.to_string(), None));
    }
    None
}

/// Authorization-code exchange body (form-urlencoded).
pub fn exchange_form(code: &str, redirect_uri: &str, client_id: &str, verifier: &str) -> String {
    format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        form_encode(code),
        form_encode(redirect_uri),
        form_encode(client_id),
        form_encode(verifier),
    )
}

/// Refresh-token grant body (form-urlencoded).
pub fn refresh_form(refresh_token: &str, client_id: &str) -> String {
    format!(
        "grant_type=refresh_token&refresh_token={}&client_id={}",
        form_encode(refresh_token),
        form_encode(client_id),
    )
}

/// Default token path: `~/.twr/oauth2.json` (separate from cookie session).
pub fn default_token_path() -> Option<std::path::PathBuf> {
    home_dir().map(|h| h.join(".twr").join("oauth2.json"))
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// Load tokens (None = missing/unparseable). Never logs token values.
pub fn load_tokens(path: &std::path::Path) -> Option<OAuth2Tokens> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Save tokens with owner-only perms (0o600) on unix. The file is CREATED
/// with mode 0o600 (bead wk1) — the old write-then-chmod left a readable
/// window and discarded chmod errors; both are propagated now.
pub fn save_tokens(path: &std::path::Path, tokens: &OAuth2Tokens) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string(tokens)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        let mut f = opts.open(path)?;
        f.write_all(raw.as_bytes())?;
        // A pre-existing file keeps its old mode — tighten it too.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, raw)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_shapes() {
        let v = new_verifier();
        assert!((43..=128).contains(&v.len()));
        let c = challenge_s256(&v);
        assert_eq!(c.len(), 43);
        // RFC 7636 Appendix B vector.
        assert_eq!(
            challenge_s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn authorize_url_has_pkce_params() {
        let url = authorize_url("cid", DEFAULT_REDIRECT_URI, DEFAULT_SCOPES, "st", "ch");
        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("code_challenge=ch"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("tweet.write"));
    }

    #[test]
    fn token_response_parses_and_expiry_anchors() {
        let body = br#"{"access_token":"a","refresh_token":"r","token_type":"Bearer","expires_in":7200,"scope":"tweet.read"}"#;
        let t = parse_token_response(body, 1000, "").unwrap();
        assert_eq!(t.access_token, "a");
        assert_eq!(t.expires_at_secs, 8200);
        assert!(!t.is_expired(8100));
        assert!(t.is_expired(8200));
        assert!(parse_token_response(b"{}", 0, "").is_err());
    }

    #[test]
    fn token_round_trip() {
        let dir = std::env::temp_dir().join(format!("twr-oauth2-{}", std::process::id()));
        let path = dir.join("oauth2.json");
        let _ = std::fs::remove_dir_all(&dir);
        let t = OAuth2Tokens {
            access_token: "a".into(),
            refresh_token: None,
            token_type: "Bearer".into(),
            expires_at_secs: 0,
            scope: "tweet.read".into(),
            client_id: Some("cid".into()),
        };
        save_tokens(&path, &t).unwrap();
        assert_eq!(load_tokens(&path).unwrap(), t);
        // Pre-p3c files without client_id still load.
        std::fs::write(
            &path,
            r#"{"access_token":"a","token_type":"Bearer","expires_at_secs":0,"scope":"s"}"#,
        )
        .unwrap();
        assert_eq!(load_tokens(&path).unwrap().client_id, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_code_accepts_bare_and_redirect_url() {
        // Bead p3c: bare code, full redirect URL, and bare query string.
        assert_eq!(extract_code("abc123"), Some(("abc123".to_string(), None)));
        assert_eq!(
            extract_code("http://127.0.0.1:8080/callback?code=xyz&state=st1"),
            Some(("xyz".to_string(), Some("st1".to_string())))
        );
        assert_eq!(
            extract_code("?code=xyz&state=st1"),
            Some(("xyz".to_string(), Some("st1".to_string())))
        );
        assert_eq!(extract_code("code=xyz"), Some(("xyz".to_string(), None)));
        assert_eq!(extract_code(""), None);
        assert_eq!(extract_code("http://x.com/no-code-here"), None);
        // "opcode=" must not false-positive as a code param.
        assert_eq!(
            extract_code("opcode=zzz"),
            Some(("opcode=zzz".to_string(), None))
        );
    }

    #[test]
    fn token_forms_are_urlencoded() {
        let f = exchange_form("c 1", "http://127.0.0.1:8080/callback", "cid", "v 2");
        assert!(f.contains("grant_type=authorization_code"));
        assert!(f.contains("code=c%201"));
        assert!(f.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8080%2Fcallback"));
        assert!(f.contains("code_verifier=v%202"));
        let r = refresh_form("rt ok", "cid");
        assert!(r.contains("grant_type=refresh_token"));
        assert!(r.contains("refresh_token=rt%20ok"));
    }
}
