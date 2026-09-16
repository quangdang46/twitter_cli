//! Read-command execution: transport → parse → envelope data.
//!
//! One function per command family, all sharing [`ExecCtx`] (auth, transport,
//! query IDs, features, transaction proof, throttle, config). Pure orchestration:
//! build variables → pick GET/POST → headers (+ gated tx-id) → parse bytes →
//! `(data, pagination)`. Error mapping goes through `twr-core` kinds.

use twr_client::{fetch_timeline, Page, PageError, Throttle};
use twr_graphql::{compact_features, ExtraRotation};
use twr_tx::RequestProof;

/// Shared execution context for all read commands.
pub struct ExecCtx<'a> {
    pub transport: &'a dyn twr_client::HttpTransport,
    pub creds: twr_client::Credentials,
    pub throttle: Throttle,
    pub extra_rotation: ExtraRotation,
    pub disk_cache: twr_graphql::cache::QueryIdCache,
    pub now_secs: u64,
    pub max_count: usize,
    #[allow(dead_code)]
    pub request_delay_secs: f64,
    pub chrome_major: String,
    pub locale: String,
    pub tx: Option<TxState>,
}

/// Live transaction-proof state (ingredients resolved once per invocation).
pub struct TxState {
    pub inner: twr_tx::ClientTransactionV1,
}

impl<'a> ExecCtx<'a> {
    /// Resolve one op's query ID through the 4 layers.
    pub fn query_id(&self, operation: &str) -> Option<twr_graphql::ResolvedQid> {
        twr_graphql::resolve(
            operation,
            |name| std::env::var(name).ok(),
            &self.disk_cache,
            &self.extra_rotation,
            self.now_secs,
        )
    }

    /// Mint a transaction id for gated ops, `None` for ungated ones.
    pub fn proof_for(&self, operation: &str, method: &str, path: &str) -> Option<String> {
        self.tx
            .as_ref()
            .and_then(|tx| tx.inner.prepare(operation, method, path))
    }
}

/// GraphQL GET URL: `/i/api/graphql/<qid>/<op>?variables=..&features=..`.
pub fn graphql_get_url(
    query_id: &str,
    operation: &str,
    variables: &serde_json::Value,
    field_toggles: Option<&serde_json::Value>,
) -> String {
    let features = serde_json::Value::Object(compact_features(operation));
    let mut url = format!(
        "https://x.com/i/api/graphql/{query_id}/{operation}?variables={}&features={}",
        url_encode(&variables.to_string()),
        url_encode(&features.to_string()),
    );
    if let Some(toggles) = field_toggles {
        url.push_str(&format!(
            "&fieldToggles={}",
            url_encode(&toggles.to_string())
        ));
    }
    url
}

fn url_encode(s: &str) -> String {
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

/// Fetch one timeline page over the transport and parse it with `twr-model`.
/// Returns `(tweets, next_cursor)`. Transport failures map to `Fatal`;
/// callers map HTTP statuses (429/404/...) via `twr-core` kinds.
pub async fn fetch_parsed_page(
    ctx: &mut ExecCtx<'_>,
    operation: &str,
    variables: serde_json::Value,
    instructions: fn(&serde_json::Value) -> Option<&Vec<serde_json::Value>>,
) -> Result<(Vec<twr_model::Tweet>, Option<String>), PageError> {
    fetch_parsed_page_with_toggles(ctx, operation, variables, instructions, None).await
}

/// Extractor: response body -> timeline instructions (per-op deep-get path).
pub type InstructionsFn = fn(&serde_json::Value) -> Option<&Vec<serde_json::Value>>;

/// Page parser: `(body, instructions) -> (items, cursor)`.
pub type PageParser<T> = fn(&serde_json::Value, InstructionsFn) -> (Vec<T>, Option<String>);

/// Generic single-page fetch: same transport/throttle/headers/status mapping
/// as [`fetch_parsed_page`], but the caller supplies the parser.
/// Lets non-tweet timelines (lists, list members) reuse the whole transport
/// layer without duplicating it — the tweet-specific wrapper below is one
/// call of this.
pub async fn fetch_parsed_page_generic<T>(
    ctx: &mut ExecCtx<'_>,
    operation: &str,
    variables: serde_json::Value,
    instructions: InstructionsFn,
    parse: PageParser<T>,
) -> Result<(Vec<T>, Option<String>), PageError> {
    let qid = ctx
        .query_id(operation)
        .map(|r| r.query_id)
        .unwrap_or_default();
    let use_post = twr_client::use_post(operation);
    let method = if use_post { "POST" } else { "GET" };
    let path = format!("/i/api/graphql/{qid}/{operation}");

    let wait = ctx.throttle.wait_secs(operation, now_f64());
    if wait > 0.0 {
        tokio::time::sleep(std::time::Duration::from_secs_f64(wait)).await;
    }

    let tid = ctx.proof_for(operation, method, &path);
    let headers = twr_client::build_headers(&twr_client::HeaderInput {
        creds: &ctx.creds,
        method,
        os: twr_client::Os::current(),
        chrome_major: &ctx.chrome_major,
        locale: &ctx.locale,
        transaction_id: tid.as_deref(),
    });
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let resp = if use_post {
        let url = format!("https://x.com/i/api/graphql/{qid}/{operation}");
        let mut body = serde_json::Map::new();
        body.insert("variables".into(), variables);
        body.insert(
            "features".into(),
            serde_json::Value::Object(compact_features(operation)),
        );
        let raw = serde_json::to_vec(&body).unwrap_or_default();
        ctx.transport
            .post_json(&url, &header_refs, &raw)
            .await
            .map_err(|_| PageError::Fatal)?
    } else {
        let url = graphql_get_url(&qid, operation, &variables, None);
        ctx.transport
            .get(&url, &header_refs)
            .await
            .map_err(|_| PageError::Fatal)?
    };

    if resp.status == 429 {
        return Err(PageError::RateLimited);
    }
    if resp.status == 404 || (500..600).contains(&resp.status) {
        return Err(PageError::Fatal);
    }
    let body: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|_| PageError::Fatal)?;
    Ok(parse(&body, instructions))
}

/// Toggle-aware variant (TweetDetail needs fieldToggles; others pass None).
#[allow(clippy::too_many_arguments)]
pub async fn fetch_parsed_page_with_toggles(
    ctx: &mut ExecCtx<'_>,
    operation: &str,
    variables: serde_json::Value,
    instructions: fn(&serde_json::Value) -> Option<&Vec<serde_json::Value>>,
    field_toggles: Option<serde_json::Value>,
) -> Result<(Vec<twr_model::Tweet>, Option<String>), PageError> {
    let qid = ctx
        .query_id(operation)
        .map(|r| r.query_id)
        .unwrap_or_default();
    let use_post = twr_client::use_post(operation);
    let method = if use_post { "POST" } else { "GET" };
    let path = format!("/i/api/graphql/{qid}/{operation}");

    // Throttle per endpoints.yaml rps/burst (best-effort wait).
    let wait = ctx.throttle.wait_secs(operation, now_f64());
    if wait > 0.0 {
        tokio::time::sleep(std::time::Duration::from_secs_f64(wait)).await;
    }

    let tid = ctx.proof_for(operation, method, &path);
    let headers = twr_client::build_headers(&twr_client::HeaderInput {
        creds: &ctx.creds,
        method,
        os: twr_client::Os::current(),
        chrome_major: &ctx.chrome_major,
        locale: &ctx.locale,
        transaction_id: tid.as_deref(),
    });
    let header_refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let resp = if use_post {
        let url = format!("https://x.com/i/api/graphql/{qid}/{operation}");
        let mut body = serde_json::Map::new();
        body.insert("variables".into(), variables);
        body.insert(
            "features".into(),
            serde_json::Value::Object(compact_features(operation)),
        );
        let raw = serde_json::to_vec(&body).unwrap_or_default();
        ctx.transport
            .post_json(&url, &header_refs, &raw)
            .await
            .map_err(|_| PageError::Fatal)?
    } else {
        let url = graphql_get_url(&qid, operation, &variables, field_toggles.as_ref());
        ctx.transport
            .get(&url, &header_refs)
            .await
            .map_err(|_| PageError::Fatal)?
    };

    if resp.status == 429 {
        return Err(PageError::RateLimited);
    }
    if resp.status == 404 || (500..600).contains(&resp.status) {
        return Err(PageError::Fatal);
    }
    let body: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|_| PageError::Fatal)?;
    // Payload-shape 404 disambiguation lives in twr-core; an *Unavailable
    // payload here parses to zero tweets (empty page), and the CLI layer
    // maps that to exit 3 when the whole result is empty.
    let (tweets, cursor) = twr_model::parse_timeline_response(&body, instructions);
    Ok((tweets, cursor))
}

/// Drive the shared [`fetch_timeline`] loop over [`fetch_parsed_page`],
/// collecting full [`twr_model::Tweet`]s with id dedup across pages.
pub async fn fetch_tweets_paged(
    ctx: &mut ExecCtx<'_>,
    operation: &str,
    count: usize,
    start_cursor: Option<String>,
    base_variables: serde_json::Value,
    instructions: fn(&serde_json::Value) -> Option<&Vec<serde_json::Value>>,
) -> Result<(Vec<twr_model::Tweet>, twr_client::TimelineResult), PageError> {
    // The loop works on IDs; we keep a side table id -> tweet.
    let mut by_id: std::collections::HashMap<String, twr_model::Tweet> =
        std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();

    // Wrap: each loop iteration fetches one page and registers its tweets.
    let ctx_ref = &mut *ctx;
    let result = fetch_timeline(
        count,
        ctx_ref.max_count,
        start_cursor,
        true,
        |page_count, cursor| {
            let mut vars = base_variables.clone();
            vars["count"] = serde_json::json!(page_count);
            if let Some(c) = cursor {
                vars["cursor"] = serde_json::json!(c);
            }
            // fetch_parsed_page is async; the loop is sync — block on it.
            // (Read commands run inside a tokio runtime; block_in_place
            // avoids nested-runtime panics.)
            let (tweets, next) = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(fetch_parsed_page(
                    ctx_ref,
                    operation,
                    vars,
                    instructions,
                ))
            })?;
            let mut page = Page {
                ids: Vec::with_capacity(tweets.len()),
                next_cursor: next,
            };
            for t in tweets {
                // Jittered inter-request delay is applied by the caller
                // between loop iterations via request_delay_secs; the
                // per-page sleep here keeps the port faithful but tiny.
                page.ids.push(t.id.clone());
                by_id.entry(t.id.clone()).or_insert(t);
            }
            // Track order of first appearance for stable output.
            for id in &page.ids {
                if !order.contains(id) {
                    order.push(id.clone());
                }
            }
            Ok(page)
        },
    )?;

    let mut tweets = Vec::new();
    for id in result.ids.iter() {
        if let Some(t) = by_id.remove(id) {
            tweets.push(t);
        }
    }
    // Preserve loop order (deduped).
    Ok((tweets, result))
}

pub fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Best-effort per-op throttle wait (bead o1l.6): consult the token bucket
/// built from `endpoints.yaml` rps/burst, sleep when it says to. Mutations
/// are one-shot (no paging loop), so without this the yaml entries for new
/// mutation ops would be dead config. Never fatal: a zero wait fires
/// immediately, and the sleep only ever delays, never fails.
pub async fn throttle_wait(throttle: &mut Throttle, operation: &str) {
    let wait = throttle.wait_secs(operation, now_f64());
    if wait > 0.0 {
        tokio::time::sleep(std::time::Duration::from_secs_f64(wait)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bead o1l.6 — every mutation op the runners throttle on must resolve
    /// to a conservative bucket (not the 1.0/4 permissive default). This
    /// pins the yaml↔code key contract: renaming a throttle call or a yaml
    /// key silently drops the op back to the default, and this test catches
    /// exactly that drift. Keys are the runner-side names (CLI op for
    /// run_engage's 1.1-REST rides, GraphQL op for post/list writes, DmSend
    /// for the DM send).
    #[test]
    fn p66_mutation_buckets_are_conservative() {
        let yaml = include_str!("../../../../endpoints/endpoints.yaml");
        let map = twr_graphql::load_yaml(Some(yaml));
        let mut missing = Vec::new();
        for op in [
            "CreateTweet",
            "CreateNoteTweet",
            "DeleteTweet",
            "FavoriteTweet",
            "UnfavoriteTweet",
            "CreateRetweet",
            "DeleteRetweet",
            "CreateBookmark",
            "DeleteBookmark",
            "like",
            "unlike",
            "retweet",
            "unretweet",
            "bookmark",
            "unbookmark",
            "follow",
            "unfollow",
            "mute",
            "unmute",
            "block",
            "unblock",
            "pin",
            "unpin",
            "delete",
            "CreateList",
            "UpdateList",
            "DeleteList",
            "ListAddMember",
            "ListRemoveMember",
            "ListSubscribe",
            "ListUnsubscribe",
            "UpdatePinnedTimelines",
            "DmSend",
        ] {
            match map.get(op) {
                Some(e) => {
                    let (rps, burst) = (e.rps.unwrap_or(99.0), e.burst.unwrap_or(99));
                    assert!(
                        rps <= 0.3 && burst <= 1,
                        "{op} bucket not conservative: rps={rps} burst={burst}"
                    );
                }
                None => missing.push(op),
            }
        }
        assert!(
            missing.is_empty(),
            "mutation ops without yaml entries: {missing:?}"
        );
    }

    #[test]
    fn graphql_get_url_encodes_variables_and_features() {
        let url = graphql_get_url(
            "qid",
            "UserByScreenName",
            &serde_json::json!({"a": 1}),
            None,
        );
        assert!(url.starts_with("https://x.com/i/api/graphql/qid/UserByScreenName?variables="));
        assert!(url.contains("features="));
        assert!(!url.contains('{'));
    }
}
