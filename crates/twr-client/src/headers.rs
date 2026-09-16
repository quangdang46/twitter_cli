//! Full request header set, ported verbatim from `client.py::_build_headers`
//! (plan §1.2 Headers).
//!
//! Every authenticated GraphQL call carries: Bearer, Cookie (full string or
//! `auth_token;ct0`), `X-Csrf-Token=ct0`, `X-Twitter-Active-User: yes`,
//! `X-Twitter-Auth-Type: OAuth2Session`, `X-Twitter-Client-Language`, an
//! OS-matched Chrome UA, Origin/Referer, dynamic `sec-ch-ua*`
//! (arch/platform), `Sec-Fetch-*`, and — for gated ops only —
//! `X-Client-Transaction-Id` minted fresh per request via `twr-tx`.
//! POST adds JSON content type + `Priority: u=1, i` and a compose Referer.

use std::collections::HashMap;

/// X's own public web-client bearer (plan §1.2). Distinct in security class
/// from user credentials — a replaceable constant, never a user secret.
pub const BEARER_TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

/// Credentials available to the header builder.
#[derive(Debug, Clone, Default)]
pub struct Credentials {
    pub auth_token: String,
    pub ct0: String,
    /// Full pasted cookie string (Method C) — wins over the pair when set.
    pub cookie_string: Option<String>,
}

impl Credentials {
    pub fn cookie_header(&self) -> String {
        match &self.cookie_string {
            Some(full) if !full.is_empty() => full.clone(),
            _ => format!("auth_token={}; ct0={}", self.auth_token, self.ct0),
        }
    }
}

/// OS the UA/client-hints describe. Matches `constants.py` platform branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Os {
    Macos,
    Windows,
    #[default]
    Linux,
}

impl Os {
    /// Detect the compile-target OS.
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Os::Macos;
        #[cfg(target_os = "windows")]
        return Os::Windows;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        return Os::Linux;
    }

    fn ua_platform(&self) -> &'static str {
        match self {
            Os::Macos => "Macintosh; Intel Mac OS X 10_15_7",
            Os::Windows => "Windows NT 10.0; Win64; x64",
            Os::Linux => "X11; Linux x86_64",
        }
    }

    fn ch_ua_platform(&self) -> &'static str {
        match self {
            Os::Macos => "\"macOS\"",
            Os::Windows => "\"Windows\"",
            Os::Linux => "\"Linux\"",
        }
    }

    fn ch_ua_platform_version(&self) -> &'static str {
        match self {
            Os::Macos => "\"15.0.0\"",
            Os::Windows => "\"10.0.0\"",
            Os::Linux => "\"\"",
        }
    }
}

/// Client-hint arch. Matches `constants.py::get_sec_ch_ua_arch`.
pub fn ch_ua_arch(machine: &str) -> &'static str {
    let m = machine.to_lowercase();
    if m.contains("arm") || m.contains("aarch") {
        "\"arm\""
    } else if m.contains("86") || m.contains("amd64") || m.contains("x64") {
        "\"x86\""
    } else {
        "\"\""
    }
}

fn host_arch() -> &'static str {
    ch_ua_arch(std::env::consts::ARCH)
}

/// Locale tag from `LC_ALL`/`LC_MESSAGES`/`LANG` (plan §1.2 Env vars),
/// e.g. `en_US.UTF-8` → `en-US`.
pub fn locale_tag(getenv: impl Fn(&str) -> Option<String>) -> String {
    let raw = getenv("LC_ALL")
        .or_else(|| getenv("LC_MESSAGES"))
        .or_else(|| getenv("LANG"))
        .unwrap_or_else(|| "en_US.UTF-8".to_string());
    let tag = raw.split('.').next().unwrap_or("").replace('_', "-");
    if tag.is_empty() {
        "en-US".to_string()
    } else {
        tag
    }
}

pub fn accept_language(tag: &str) -> String {
    let language = tag.split('-').next().unwrap_or("en");
    let language = if language.is_empty() { "en" } else { language };
    format!("{tag},{language};q=0.9,en;q=0.8")
}

pub fn client_language(tag: &str) -> String {
    let lang = tag.split('-').next().unwrap_or("en");
    if lang.is_empty() {
        "en".to_string()
    } else {
        lang.to_string()
    }
}

/// Inputs to one header build.
pub struct HeaderInput<'a> {
    pub creds: &'a Credentials,
    pub method: &'a str,
    pub os: Os,
    pub chrome_major: &'a str,
    pub locale: &'a str,
    /// Fresh transaction id for gated ops; `None` for ungated ones (the
    /// caller decides via `twr-tx`'s `RequestProof::prepare`).
    pub transaction_id: Option<&'a str>,
}

/// Build the full header map. Mirrors `_build_headers` key-for-key.
pub fn build_headers(input: &HeaderInput) -> HashMap<String, String> {
    let mut h = HashMap::new();
    h.insert("Authorization".into(), format!("Bearer {BEARER_TOKEN}"));
    h.insert("Cookie".into(), input.creds.cookie_header());
    h.insert("X-Csrf-Token".into(), input.creds.ct0.clone());
    h.insert("X-Twitter-Active-User".into(), "yes".into());
    h.insert("X-Twitter-Auth-Type".into(), "OAuth2Session".into());
    h.insert(
        "X-Twitter-Client-Language".into(),
        client_language(input.locale),
    );
    h.insert(
        "User-Agent".into(),
        format!(
            "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36",
            input.os.ua_platform(),
            input.chrome_major,
        ),
    );
    h.insert("Origin".into(), "https://x.com".into());
    h.insert("Referer".into(), "https://x.com/".into());
    h.insert("Accept".into(), "*/*".into());
    h.insert("Accept-Language".into(), accept_language(input.locale));
    h.insert(
        "sec-ch-ua".into(),
        format!(
            "\"Chromium\";v=\"{}\", \"Not(A:Brand\";v=\"99\", \"Google Chrome\";v=\"{}\"",
            input.chrome_major, input.chrome_major,
        ),
    );
    h.insert("sec-ch-ua-mobile".into(), "?0".into());
    h.insert(
        "sec-ch-ua-platform".into(),
        input.os.ch_ua_platform().into(),
    );
    h.insert("sec-ch-ua-arch".into(), host_arch().into());
    h.insert("sec-ch-ua-bitness".into(), "\"64\"".into());
    h.insert(
        "sec-ch-ua-full-version".into(),
        format!("\"{}.0.0.0\"", input.chrome_major),
    );
    h.insert(
        "sec-ch-ua-full-version-list".into(),
        format!(
            "\"Google Chrome\";v=\"{0}.0.0.0\", \"Chromium\";v=\"{0}.0.0.0\", \"Not.A/Brand\";v=\"99.0.0.0\"",
            input.chrome_major,
        ),
    );
    h.insert("sec-ch-ua-model".into(), "\"\"".into());
    h.insert(
        "sec-ch-ua-platform-version".into(),
        input.os.ch_ua_platform_version().into(),
    );
    h.insert("Sec-Fetch-Dest".into(), "empty".into());
    h.insert("Sec-Fetch-Mode".into(), "cors".into());
    h.insert("Sec-Fetch-Site".into(), "same-origin".into());

    if input.method == "POST" {
        h.insert("Content-Type".into(), "application/json".into());
        h.insert("Referer".into(), "https://x.com/compose/post".into());
        h.insert("Priority".into(), "u=1, i".into());
    }
    if let Some(tid) = input.transaction_id {
        h.insert("X-Client-Transaction-Id".into(), tid.to_string());
    }
    h
}

/// Query params for the URT notifications REST endpoint
/// (`GET https://x.com/i/api/2/notifications/{all,mentions}.json`), ported
/// verbatim from xfetch's `NotificationsMixin::fetchNotifications`: the
/// `include_*` bundle, `tweet_mode=extended`, `count`, and the `ext`
/// bundle. `cursor` is appended only when paging.
pub fn notification_params(count: usize, cursor: Option<&str>) -> Vec<(String, String)> {
    let mut p: Vec<(String, String)> = vec![
        ("include_profile_interstitial_type", "1"),
        ("include_blocking", "1"),
        ("include_blocked_by", "1"),
        ("include_followed_by", "1"),
        ("include_want_retweets", "1"),
        ("include_mute_edge", "1"),
        ("include_can_dm", "1"),
        ("include_can_media_tag", "1"),
        ("include_ext_is_blue_verified", "1"),
        ("include_ext_verified_type", "1"),
        ("include_ext_profile_image_shape", "1"),
        ("skip_status", "1"),
        ("cards_platform", "Web-12"),
        ("include_cards", "1"),
        ("include_ext_alt_text", "true"),
        ("include_ext_limited_action_results", "true"),
        ("include_quote_count", "true"),
        ("include_reply_count", "1"),
        ("tweet_mode", "extended"),
        ("include_ext_views", "true"),
        ("include_entities", "true"),
        ("include_user_entities", "true"),
        ("include_ext_media_color", "true"),
        ("include_ext_media_availability", "true"),
        ("include_ext_sensitive_media_warning", "true"),
        ("include_ext_trusted_friends_metadata", "true"),
        ("send_error_codes", "true"),
        ("simple_quoted_tweet", "true"),
        (
            "ext",
            "mediaStats,highlightedLabel,voiceInfo,birdwatchPivot,superFollowMetadata,unmentionInfo,editControl",
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    p.push(("count".to_string(), count.to_string()));
    if let Some(c) = cursor.filter(|c| !c.is_empty()) {
        p.push(("cursor".to_string(), c.to_string()));
    }
    p
}

/// Full notifications URL with the param bundle encoded.
pub fn notifications_url(kind: &str, count: usize, cursor: Option<&str>) -> String {
    let qs: Vec<String> = notification_params(count, cursor)
        .iter()
        .map(|(k, v)| format!("{k}={}", url_encode_query(v)))
        .collect();
    format!(
        "https://x.com/i/api/2/notifications/{kind}.json?{}",
        qs.join("&")
    )
}

fn url_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> Credentials {
        Credentials {
            auth_token: "tok".into(),
            ct0: "ct".into(),
            cookie_string: None,
        }
    }

    fn input<'a>(creds: &'a Credentials, method: &'a str, tid: Option<&'a str>) -> HeaderInput<'a> {
        HeaderInput {
            creds,
            method,
            os: Os::Linux,
            chrome_major: "133",
            locale: "en-US",
            transaction_id: tid,
        }
    }

    #[test]
    fn notification_params_carry_xfetch_bundle_and_count() {
        let p = notification_params(20, None);
        let get = |k: &str| p.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.as_str());
        assert_eq!(get("tweet_mode"), Some("extended"));
        assert_eq!(get("count"), Some("20"));
        assert_eq!(get("cursor"), None);
        assert!(get("ext").is_some_and(|e| e.contains("mediaStats")));
        let paged = notification_params(50, Some("CURSOR"));
        let get2 = |k: &str| {
            paged
                .iter()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get2("cursor"), Some("CURSOR"));
        let url = notifications_url("mentions", 20, None);
        assert!(url.starts_with("https://x.com/i/api/2/notifications/mentions.json?"));
        assert!(url.contains("count=20"));
    }

    #[test]
    fn bearer_cookie_csrf_and_auth_type_present() {
        let c = creds();
        let h = build_headers(&input(&c, "GET", None));
        assert!(h["Authorization"].starts_with("Bearer AAAA"));
        assert_eq!(h["Cookie"], "auth_token=tok; ct0=ct");
        assert_eq!(h["X-Csrf-Token"], "ct");
        assert_eq!(h["X-Twitter-Active-User"], "yes");
        assert_eq!(h["X-Twitter-Auth-Type"], "OAuth2Session");
    }

    #[test]
    fn full_cookie_string_wins_over_pair() {
        let c = Credentials {
            cookie_string: Some("auth_token=a; ct0=b; guest_id=c".into()),
            ..creds()
        };
        let h = build_headers(&input(&c, "GET", None));
        assert_eq!(h["Cookie"], "auth_token=a; ct0=b; guest_id=c");
    }

    #[test]
    fn post_adds_json_priority_and_compose_referer() {
        let c = creds();
        let get = build_headers(&input(&c, "GET", None));
        assert!(!get.contains_key("Content-Type"));
        assert!(!get.contains_key("Priority"));
        let post = build_headers(&input(&c, "POST", None));
        assert_eq!(post["Content-Type"], "application/json");
        assert_eq!(post["Priority"], "u=1, i");
        assert_eq!(post["Referer"], "https://x.com/compose/post");
    }

    #[test]
    fn transaction_id_attached_only_when_given() {
        let c = creds();
        assert!(!build_headers(&input(&c, "GET", None)).contains_key("X-Client-Transaction-Id"));
        let h = build_headers(&input(&c, "GET", Some("tid-1")));
        assert_eq!(h["X-Client-Transaction-Id"], "tid-1");
    }

    #[test]
    fn locale_tag_follows_lc_all_then_lang() {
        let tag = locale_tag(|k| match k {
            "LC_ALL" => Some("fr_FR.UTF-8".into()),
            _ => None,
        });
        assert_eq!(tag, "fr-FR");
        assert_eq!(accept_language(&tag), "fr-FR,fr;q=0.9,en;q=0.8");
        assert_eq!(client_language(&tag), "fr");
        let fallback = locale_tag(|_| None);
        assert_eq!(fallback, "en-US");
    }

    #[test]
    fn ua_matches_os() {
        let c = creds();
        let mac = HeaderInput {
            os: Os::Macos,
            ..input(&c, "GET", None)
        };
        assert!(build_headers(&mac)["User-Agent"].contains("Macintosh"));
        assert_eq!(build_headers(&mac)["sec-ch-ua-platform"], "\"macOS\"");
    }
}
