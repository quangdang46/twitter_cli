//! Read a home timeline page over the library API (no CLI).
//!
//! ```sh
//! TWITTER_AUTH_TOKEN=… TWITTER_CT0=… cargo run --example read_timeline
//! ```
//! Needs real credentials; fails with exit 77 guidance without them.

use twr_auth::{read_env, resolve, FlagInput};
use twr_client::{build_headers, Credentials, HeaderInput, Os, WreqTransport};
use twr_graphql::{compact_features, resolve as resolve_qid, ExtraRotation};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Auth: flags (empty) → env → file → browser.
    let env = read_env();
    let file = std::env::var_os("HOME")
        .map(|h| {
            std::path::PathBuf::from(h)
                .join(".twr")
                .join("session.json")
        })
        .and_then(|p| twr_auth::load_session(&p));
    let auth = resolve(&FlagInput::default(), &env, file, || {
        (twr_auth::SessionCookies::default(), vec![])
    });
    let Some(auth) = auth else {
        eprintln!("no session: run `twr login --guide` first (exit 77)");
        std::process::exit(77);
    };

    // 2. Query ID through the 4 layers.
    let disk = std::collections::HashMap::new();
    let extra = ExtraRotation::new();
    let qid = resolve_qid("HomeTimeline", |n| std::env::var(n).ok(), &disk, &extra, 0)
        .map(|r| r.query_id)
        .unwrap_or_default();

    // 3. One GET page over the impersonating transport.
    let transport = WreqTransport::new_chrome().expect("transport builds");
    let creds = Credentials {
        auth_token: auth.session.auth_token.unwrap_or_default(),
        ct0: auth.session.ct0.unwrap_or_default(),
        cookie_string: None,
    };
    let locale = twr_client::headers::locale_tag(|k| std::env::var(k).ok());
    let headers = build_headers(&HeaderInput {
        creds: &creds,
        method: "GET",
        os: Os::current(),
        chrome_major: "133",
        locale: &locale,
        transaction_id: None,
    });
    let refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let features = serde_json::Value::Object(compact_features("HomeTimeline"));
    let url = format!(
        "https://x.com/i/api/graphql/{qid}/HomeTimeline?variables=%7B%22count%22%3A20%7D&features={}",
        serde_json::to_string(&features).unwrap_or_default()
    );
    let resp = twr_client::HttpTransport::get(&transport, &url, &refs).await?;
    println!("HTTP {} ({} bytes)", resp.status, resp.body.len());
    Ok(())
}
