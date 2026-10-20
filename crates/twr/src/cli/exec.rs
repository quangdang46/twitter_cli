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
    /// Where freshly derived ingredients are persisted; `None` skips caching.
    pub tx_cache_path: Option<std::path::PathBuf>,
    /// True while `tx` came from the disk cache rather than from a derivation
    /// in this run; only then can a rejection mean the cached material is bad.
    pub tx_from_cache: bool,
    /// Set once this run tried to derive, so a failing X is hit at most once.
    pub tx_attempted: bool,
    /// Why the last derivation failed; reported when a gated op then fails.
    pub tx_failure: Option<String>,
    pub verbose: bool,
    /// Sink for `-v` diagnostics (stderr in production).
    pub diag: std::sync::Arc<dyn Fn(&str) + Send + Sync>,
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

    /// Derive the transaction ingredients on first use of a gated op when the
    /// disk cache had none. Failure leaves `tx` unset: the request goes out
    /// without the header, as it did before derivation existed.
    pub async fn ensure_tx(&mut self, operation: &str) {
        if self.tx.is_some() || self.tx_attempted || !twr_tx::GATED_OPS.contains(&operation) {
            return;
        }
        self.tx_attempted = true;
        match self.derive_tx_ingredients().await {
            Ok(ingredients) => {
                self.install_ingredients(operation, ingredients);
            }
            Err(why) => self.fail_tx(operation, why),
        }
    }

    /// Use freshly derived ingredients and cache them. Unusable ones are
    /// rejected before they reach the cache, where they would be served for
    /// the next hour.
    fn install_ingredients(
        &mut self,
        operation: &str,
        ingredients: twr_tx::cache::CachedIngredients,
    ) {
        let key_bytes = match ingredients.key_bytes() {
            Ok(key_bytes) => key_bytes,
            Err(why) => return self.fail_tx(operation, why),
        };
        if let Some(path) = &self.tx_cache_path {
            if let Err(e) = twr_tx::cache::store(path, &ingredients) {
                self.note(&format!(
                    "twr: could not cache the transaction ingredients at {} ({e})",
                    path.display()
                ));
            }
        }
        self.tx_failure = None;
        self.tx_from_cache = false;
        self.tx = Some(TxState {
            inner: twr_tx::ClientTransactionV1::new(key_bytes, ingredients.animation_key),
        });
    }

    fn fail_tx(&mut self, operation: &str, why: String) {
        self.note(&format!(
            "twr: no x-client-transaction-id for {operation} ({why})"
        ));
        self.tx_failure = Some(why);
    }

    /// Drop cached ingredients and derive them again. A gated op that was
    /// rejected despite its header may have been signed with stale or wrong
    /// material, which the 1h cache would otherwise keep serving. Ingredients
    /// derived in this run are not re-fetched: the next id is just a new one.
    pub async fn refresh_tx(&mut self, operation: &str) {
        if !twr_tx::GATED_OPS.contains(&operation) || !self.tx_from_cache {
            return;
        }
        if let Some(path) = &self.tx_cache_path {
            let _ = std::fs::remove_file(path);
        }
        self.tx = None;
        self.ensure_tx(operation).await;
    }

    fn note(&self, line: &str) {
        if self.verbose {
            (self.diag)(line);
        }
    }

    async fn derive_tx_ingredients(&self) -> Result<twr_tx::cache::CachedIngredients, String> {
        let cookie = self.creds.cookie_header();
        let language = twr_client::headers::accept_language(&self.locale);
        let home = self
            .transport
            .get(
                "https://x.com/home",
                &[
                    ("Cookie", cookie.as_str()),
                    ("Accept", "text/html,application/xhtml+xml"),
                    ("Accept-Language", language.as_str()),
                ],
            )
            .await
            .map_err(|e| format!("home page: {e}"))?;
        if home.status != 200 {
            return Err(format!("home page: HTTP {}", home.status));
        }
        let home_html = String::from_utf8_lossy(&home.body);
        let chunk_url = twr_tx::find_ondemand_url(&home_html).map_err(|e| e.to_string())?;
        // The chunk is static CDN content: no cookie goes to a third-party host.
        let chunk = self
            .transport
            .get(&chunk_url, &[])
            .await
            .map_err(|e| format!("ondemand.s: {e}"))?;
        if chunk.status != 200 {
            return Err(format!("ondemand.s: HTTP {}", chunk.status));
        }
        twr_tx::derive_ingredients(
            &home_html,
            &String::from_utf8_lossy(&chunk.body),
            self.now_secs,
        )
        .map_err(|e| e.to_string())
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

    ctx.ensure_tx(operation).await;
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

    ctx.ensure_tx(operation).await;
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

/// Result of [`post_engage_graphql`]: the final response, and whether it
/// came from the single resend after a gated op was rejected.
pub struct EngagePost {
    pub response: twr_client::TransportResponse,
    pub resent: bool,
    /// Whether the first request carried an id; a failed refresh can leave
    /// `ctx.tx` empty although that request was signed.
    pub first_had_tx: bool,
}

/// POST one GraphQL engagement mutation (like / bookmark / retweet / pin …).
/// Gated ops (`CreateBookmark`, `DeleteBookmark`) carry a freshly minted
/// `x-client-transaction-id`; X answers them 404 without it. X also 404s a
/// small share of requests that do carry one, and a cached key that went
/// stale does the same, so a gated 404 is resent once with a new id (and
/// re-derived ingredients when the old ones came from the disk cache).
///
/// The resend is a second write attempt that the caller's budget count does
/// not see; it goes through the throttle under `throttle_key` like the first.
pub async fn post_engage_graphql(
    ctx: &mut ExecCtx<'_>,
    op: &str,
    target_id: &str,
    throttle_key: &str,
) -> Result<EngagePost, twr_client::TransportError> {
    ctx.ensure_tx(op).await;
    let first_had_tx = ctx.tx.is_some();
    let mut response = send_engage(ctx, op, target_id).await?;
    let mut resent = false;
    if response.status == 404 && twr_tx::GATED_OPS.contains(&op) {
        ctx.refresh_tx(op).await;
        // Without an id the resend would be the same rejected request.
        if ctx.tx.is_some() {
            throttle_wait(&mut ctx.throttle, throttle_key).await;
            response = send_engage(ctx, op, target_id).await?;
            resent = true;
        }
    }
    Ok(EngagePost {
        response,
        resent,
        first_had_tx,
    })
}

async fn send_engage(
    ctx: &ExecCtx<'_>,
    op: &str,
    target_id: &str,
) -> Result<twr_client::TransportResponse, twr_client::TransportError> {
    let qid = ctx.query_id(op).map(|r| r.query_id).unwrap_or_default();
    let path = format!("/i/api/graphql/{qid}/{op}");
    let headers = twr_client::build_headers(&twr_client::HeaderInput {
        creds: &ctx.creds,
        method: "POST",
        os: twr_client::Os::current(),
        chrome_major: &ctx.chrome_major,
        locale: &ctx.locale,
        transaction_id: ctx.proof_for(op, "POST", &path).as_deref(),
    });
    let refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let vars = if op == "PinTweet" || op == "UnpinTweet" {
        super::write::pin_vars(target_id)
    } else {
        super::write::tweet_id_vars(op, target_id)
    };
    let mut body = serde_json::Map::new();
    body.insert("variables".into(), vars);
    body.insert(
        "features".into(),
        serde_json::Value::Object(compact_features(op)),
    );
    let raw = serde_json::to_vec(&body).unwrap_or_default();
    ctx.transport
        .post_json(&format!("https://x.com{path}"), &refs, &raw)
        .await
}

/// Error data for a non-2xx engagement response. A 404 on a gated op has two
/// possible causes (a missing or wrong transaction id, or a stale query id),
/// so the message says what was actually sent and the suggestion names both.
pub fn engage_failure_data(
    ctx: &ExecCtx<'_>,
    cmd: &str,
    op: &str,
    post: &EngagePost,
) -> serde_json::Value {
    let status = post.response.status;
    if status != 404 || !twr_tx::GATED_OPS.contains(&op) {
        return serde_json::json!({"error": format!("{cmd} failed: HTTP {status}")});
    }
    let why = ctx.tx_failure.as_deref().unwrap_or("not derived");
    let error = if post.resent {
        format!("{cmd} failed: HTTP 404 even with an x-client-transaction-id (resent once with a new id)")
    } else if post.first_had_tx {
        format!("{cmd} failed: HTTP 404 with an x-client-transaction-id; deriving new ingredients failed, so it was not resent ({why})")
    } else {
        format!("{cmd} failed: HTTP 404, no x-client-transaction-id was sent ({why})")
    };
    serde_json::json!({
        "error": error,
        "suggestion": "retry once; if it keeps failing, either the query id is stale (run `twr doctor --refresh`) or the x-client-transaction-id derivation has drifted and needs re-porting",
    })
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

    type Seen = Vec<(String, Vec<(String, String)>)>;

    /// Serves the home page and the ondemand chunk from fixtures and records
    /// every request, so no test reaches the network.
    struct FakeX {
        home_status: u16,
        chunk_status: u16,
        /// Statuses for successive graphql requests; 200 once exhausted.
        graphql_script: std::sync::Mutex<std::collections::VecDeque<u16>>,
        gets: std::sync::Mutex<Seen>,
        posts: std::sync::Mutex<Seen>,
    }

    impl FakeX {
        fn new(home_status: u16) -> Self {
            Self {
                home_status,
                chunk_status: 200,
                graphql_script: Default::default(),
                gets: Default::default(),
                posts: Default::default(),
            }
        }

        fn with_graphql(statuses: &[u16]) -> Self {
            let x = Self::new(200);
            x.graphql_script.lock().unwrap().extend(statuses);
            x
        }

        fn graphql_status(&self) -> u16 {
            self.graphql_script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(200)
        }
    }

    fn record(into: &std::sync::Mutex<Seen>, url: &str, headers: &[(&str, &str)]) {
        into.lock().unwrap().push((
            url.to_string(),
            headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ));
    }

    fn header<'a>(seen: &'a (String, Vec<(String, String)>), name: &str) -> Option<&'a str> {
        seen.1
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn has_header(seen: &(String, Vec<(String, String)>), name: &str) -> bool {
        header(seen, name).is_some()
    }

    #[async_trait::async_trait]
    impl twr_client::HttpTransport for FakeX {
        async fn get(
            &self,
            url: &str,
            headers: &[(&str, &str)],
        ) -> Result<twr_client::TransportResponse, twr_client::TransportError> {
            record(&self.gets, url, headers);
            let (status, body) = if url == "https://x.com/home" {
                (self.home_status, home_html())
            } else if url.starts_with("https://abs.twimg.com/") {
                (
                    self.chunk_status,
                    "x(a[24], 16)*(a[29], 16)*(a[31], 16)*(a[22], 16)".into(),
                )
            } else {
                (self.graphql_status(), "{}".into())
            };
            Ok(twr_client::TransportResponse {
                status,
                body: body.into_bytes(),
            })
        }

        async fn post_json(
            &self,
            url: &str,
            headers: &[(&str, &str)],
            _body: &[u8],
        ) -> Result<twr_client::TransportResponse, twr_client::TransportError> {
            record(&self.posts, url, headers);
            Ok(twr_client::TransportResponse {
                status: self.graphql_status(),
                body: b"{}".to_vec(),
            })
        }
    }

    // Same synthetic page as twr-tx's tests: key bytes 0..48, so the derived
    // animation key is the one pinned there against the Python reference.
    const PYTHON_ANIMATION_KEY: &str =
        "58595a0ee147ae147ae1805c28f5c28f5c2805c28f5c28f5c280ee147ae147ae1800";

    fn home_html() -> String {
        use base64::Engine as _;
        let key = base64::engine::general_purpose::STANDARD.encode((0u8..48).collect::<Vec<_>>());
        let rows: Vec<String> = (0..12)
            .map(|row| {
                (0..11)
                    .map(|n| ((row * 11 + n) % 256).to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        let d = format!("M0 0 C 0 {}", rows.join("C"));
        let frames: String = (0..4)
            .map(|i| {
                format!(
                    r#"<svg id="loading-x-anim-{i}"><g><path d="M1 1"/><path d="{d}"/></g></svg>"#
                )
            })
            .collect();
        format!(
            r#"<html><head><meta name="twitter-site-verification" content="{key}"/></head><body>{frames}<script>a={{59924:"ondemand.s",7:"other"}},b={{59924:"deadbeef",122:"aaaa"}}</script></body></html>"#
        )
    }

    /// Removes the directory on drop, so a failing assertion leaves nothing.
    struct TmpDir(std::path::PathBuf);

    impl TmpDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("twr-exec-tx-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }

        fn cache_path(&self) -> std::path::PathBuf {
            self.0.join("transaction_cache.json")
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_ctx<'a>(transport: &'a FakeX, tmp: &TmpDir) -> ExecCtx<'a> {
        ExecCtx {
            transport,
            creds: twr_client::Credentials {
                auth_token: "at".into(),
                ct0: "ct".into(),
                cookie_string: Some("auth_token=at; ct0=ct; kdt=full".into()),
            },
            throttle: Throttle::new(twr_client::BucketConfig {
                rps: 1000.0,
                burst: 1000.0,
            }),
            extra_rotation: twr_graphql::seeded_extra_rotation(),
            disk_cache: Default::default(),
            now_secs: 1_800_000_000,
            max_count: 200,
            request_delay_secs: 0.0,
            chrome_major: "133".into(),
            locale: "en-US".into(),
            tx: None,
            tx_cache_path: Some(tmp.cache_path()),
            tx_from_cache: false,
            tx_attempted: false,
            tx_failure: None,
            verbose: false,
            diag: std::sync::Arc::new(|_| {}),
        }
    }

    /// Same as `test_ctx`, collecting the `-v` lines.
    fn verbose_ctx<'a>(
        transport: &'a FakeX,
        tmp: &TmpDir,
    ) -> (ExecCtx<'a>, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = lines.clone();
        let mut ctx = test_ctx(transport, tmp);
        ctx.verbose = true;
        ctx.diag = std::sync::Arc::new(move |l| sink.lock().unwrap().push(l.to_string()));
        (ctx, lines)
    }

    /// Takes a minted id apart: `(key bytes, unix-ish time, sha256 prefix)`.
    fn decode_tid(tid: &str) -> (Vec<u8>, i64, Vec<u8>) {
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(tid)
            .unwrap();
        let noise = raw[0];
        let payload: Vec<u8> = raw[1..].iter().map(|b| b ^ noise).collect();
        let key_len = payload.len() - 4 - 16 - 1;
        let t = &payload[key_len..key_len + 4];
        let time =
            i64::from(t[0]) | i64::from(t[1]) << 8 | i64::from(t[2]) << 16 | i64::from(t[3]) << 24;
        (
            payload[..key_len].to_vec(),
            time,
            payload[key_len + 4..key_len + 20].to_vec(),
        )
    }

    /// True when `tid` is the id for exactly this method + path under the
    /// reference animation key and the reference key bytes.
    fn tid_signs(tid: &str, method: &str, path: &str) -> bool {
        use sha2::{Digest, Sha256};
        let (key, time, digest) = decode_tid(tid);
        let msg = format!("{method}!{path}!{time}obfiowerehiring{PYTHON_ANIMATION_KEY}");
        key == (0u8..48).collect::<Vec<_>>() && digest == Sha256::digest(msg)[..16]
    }

    fn path_of(url: &str) -> &str {
        url.strip_prefix("https://x.com").unwrap()
    }

    #[tokio::test]
    async fn ensure_tx_derives_from_the_fetched_pages_and_caches_the_ingredients() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("derive");
        let mut ctx = test_ctx(&x, &tmp);
        ctx.ensure_tx("CreateBookmark").await;

        assert!(ctx.tx.is_some());
        let gets = x.gets.lock().unwrap();
        assert_eq!(gets.len(), 2);
        assert_eq!(gets[0].0, "https://x.com/home");
        assert!(header(&gets[0], "Cookie").unwrap().contains("kdt=full"));
        assert_eq!(
            gets[1].0,
            "https://abs.twimg.com/responsive-web/client-web/ondemand.s.deadbeefa.js"
        );
        assert!(!has_header(&gets[1], "Cookie"));

        let cached = twr_tx::cache::load_fresh(&tmp.cache_path(), ctx.now_secs).unwrap();
        assert_eq!(cached.animation_key, PYTHON_ANIMATION_KEY);
        assert_eq!(cached.key_bytes().unwrap(), (0u8..48).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn ensure_tx_leaves_ungated_ops_alone() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("ungated");
        let mut ctx = test_ctx(&x, &tmp);
        ctx.ensure_tx("FavoriteTweet").await;

        assert!(ctx.tx.is_none());
        assert!(x.gets.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ensure_tx_failure_keeps_going_without_tx_and_does_not_retry() {
        let x = FakeX::new(500);
        let tmp = TmpDir::new("fail");
        let mut ctx = test_ctx(&x, &tmp);
        ctx.ensure_tx("CreateBookmark").await;
        ctx.ensure_tx("CreateBookmark").await;

        assert!(ctx.tx.is_none());
        assert_eq!(ctx.tx_failure.as_deref(), Some("home page: HTTP 500"));
        assert_eq!(x.gets.lock().unwrap().len(), 1);
        assert!(!tmp.cache_path().exists());
    }

    #[tokio::test]
    async fn ensure_tx_treats_a_failing_chunk_fetch_like_any_other_failure() {
        let x = FakeX {
            chunk_status: 404,
            ..FakeX::new(200)
        };
        let tmp = TmpDir::new("chunk-fail");
        let mut ctx = test_ctx(&x, &tmp);
        ctx.ensure_tx("CreateBookmark").await;

        assert!(ctx.tx.is_none());
        assert_eq!(ctx.tx_failure.as_deref(), Some("ondemand.s: HTTP 404"));
        assert_eq!(x.gets.lock().unwrap().len(), 2);
        assert!(!tmp.cache_path().exists());
    }

    #[tokio::test]
    async fn verbose_reports_a_failed_derivation_in_one_line_and_quiet_stays_silent() {
        let x = FakeX::new(500);
        let tmp = TmpDir::new("verbose");
        let (mut ctx, lines) = verbose_ctx(&x, &tmp);
        ctx.ensure_tx("CreateBookmark").await;
        assert_eq!(
            *lines.lock().unwrap(),
            vec!["twr: no x-client-transaction-id for CreateBookmark (home page: HTTP 500)"]
        );

        let tmp = TmpDir::new("quiet");
        let mut quiet = test_ctx(&x, &tmp);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(0));
        let counter = seen.clone();
        quiet.diag = std::sync::Arc::new(move |_| *counter.lock().unwrap() += 1);
        quiet.ensure_tx("CreateBookmark").await;
        assert_eq!(*seen.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_cache_that_cannot_be_written_is_reported_and_the_run_continues() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("unwritable");
        std::fs::create_dir_all(&tmp.0).unwrap();
        // A regular file where the cache directory should be.
        let blocker = tmp.0.join("blocker");
        std::fs::write(&blocker, "").unwrap();
        let (mut ctx, lines) = verbose_ctx(&x, &tmp);
        ctx.tx_cache_path = Some(blocker.join("transaction_cache.json"));
        ctx.ensure_tx("CreateBookmark").await;

        assert!(
            ctx.tx.is_some(),
            "an unwritable cache must not lose the ids"
        );
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("twr: could not cache the transaction ingredients at "));
    }

    /// As `build_ctx` leaves things after loading a fresh but wrong entry.
    fn load_stale_cache(ctx: &mut ExecCtx<'_>, tmp: &TmpDir) {
        let stale = twr_tx::cache::CachedIngredients::new(&[9u8; 48], "stale", ctx.now_secs);
        twr_tx::cache::store(&tmp.cache_path(), &stale).unwrap();
        ctx.tx = Some(TxState {
            inner: twr_tx::ClientTransactionV1::new(vec![9u8; 48], "stale".into()),
        });
        ctx.tx_from_cache = true;
    }

    #[tokio::test]
    async fn refresh_tx_discards_the_cached_ingredients_and_derives_again() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("refresh");
        let mut ctx = test_ctx(&x, &tmp);
        load_stale_cache(&mut ctx, &tmp);
        ctx.refresh_tx("CreateBookmark").await;

        assert!(!ctx.tx_from_cache, "now derived in this run");
        let cached = twr_tx::cache::load_fresh(&tmp.cache_path(), ctx.now_secs).unwrap();
        assert_eq!(cached.animation_key, PYTHON_ANIMATION_KEY);
        let tid = ctx.proof_for("CreateBookmark", "POST", "/p").unwrap();
        assert!(tid_signs(&tid, "POST", "/p"));
    }

    #[tokio::test]
    async fn refresh_tx_removes_the_cache_even_when_deriving_again_fails() {
        let x = FakeX::new(500);
        let tmp = TmpDir::new("refresh-fail");
        let mut ctx = test_ctx(&x, &tmp);
        load_stale_cache(&mut ctx, &tmp);
        ctx.refresh_tx("CreateBookmark").await;
        // The failed round is not repeated for the same run.
        ctx.refresh_tx("CreateBookmark").await;

        assert!(ctx.tx.is_none());
        assert!(!tmp.cache_path().exists());
        assert_eq!(x.gets.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn refresh_tx_does_not_refetch_what_this_run_just_derived() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("refresh-fresh");
        let mut ctx = test_ctx(&x, &tmp);
        ctx.ensure_tx("CreateBookmark").await;
        ctx.refresh_tx("CreateBookmark").await;

        assert!(ctx.tx.is_some());
        assert_eq!(
            x.gets.lock().unwrap().len(),
            2,
            "no second home + chunk fetch"
        );
        assert!(tmp.cache_path().exists());
    }

    #[tokio::test]
    async fn ingredients_with_an_unusable_key_are_reported_and_never_cached() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("bad-key");
        let mut ctx = test_ctx(&x, &tmp);
        let broken = twr_tx::cache::CachedIngredients {
            key_b64: "***not base64***".into(),
            animation_key: "k".into(),
            derived_at_secs: ctx.now_secs,
        };
        ctx.install_ingredients("CreateBookmark", broken);

        assert!(ctx.tx.is_none());
        assert!(ctx
            .tx_failure
            .as_deref()
            .unwrap()
            .contains("not valid base64"));
        assert!(!tmp.cache_path().exists());
    }

    #[tokio::test]
    async fn engage_post_signs_each_bookmark_id_with_its_own_post_and_path() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("bookmark");
        let mut ctx = test_ctx(&x, &tmp);
        for op in ["CreateBookmark", "DeleteBookmark"] {
            let post = post_engage_graphql(&mut ctx, op, "123", "k").await.unwrap();
            assert!(!post.resent);
        }

        let posts = x.posts.lock().unwrap();
        assert_eq!(posts.len(), 2);
        for (post, op) in posts.iter().zip(["CreateBookmark", "DeleteBookmark"]) {
            assert!(post.0.ends_with(&format!("/{op}")));
            let tid = header(post, "X-Client-Transaction-Id").expect("tx id header");
            assert!(tid_signs(tid, "POST", path_of(&post.0)), "{op}: {tid}");
            // The id is bound to this op's path, not to any other request.
            assert!(!tid_signs(tid, "GET", path_of(&post.0)));
            assert!(!tid_signs(tid, "POST", "/i/api/graphql/x/Other"));
        }
        assert_eq!(x.gets.lock().unwrap().len(), 2, "derived once, reused");
    }

    #[tokio::test]
    async fn engage_post_keeps_the_header_off_ungated_ops() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("like");
        let mut ctx = test_ctx(&x, &tmp);
        post_engage_graphql(&mut ctx, "FavoriteTweet", "123", "k")
            .await
            .unwrap();

        let posts = x.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert!(!has_header(&posts[0], "X-Client-Transaction-Id"));
        assert!(x.gets.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn engage_post_still_goes_out_without_the_header_when_derivation_fails() {
        let x = FakeX::new(500);
        let tmp = TmpDir::new("bookmark-fail");
        let mut ctx = test_ctx(&x, &tmp);
        let post = post_engage_graphql(&mut ctx, "CreateBookmark", "123", "k")
            .await
            .unwrap();

        assert_eq!(post.response.status, 200);
        let posts = x.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert!(!has_header(&posts[0], "X-Client-Transaction-Id"));
    }

    #[tokio::test]
    async fn a_gated_404_on_cached_ingredients_is_resent_once_with_re_derived_ones() {
        let x = FakeX::with_graphql(&[404, 200]);
        let tmp = TmpDir::new("resend");
        let mut ctx = test_ctx(&x, &tmp);
        load_stale_cache(&mut ctx, &tmp);
        let post = post_engage_graphql(&mut ctx, "CreateBookmark", "123", "k")
            .await
            .unwrap();

        assert_eq!(post.response.status, 200);
        assert!(post.resent);
        let posts = x.posts.lock().unwrap();
        assert_eq!(posts.len(), 2);
        let first = header(&posts[0], "X-Client-Transaction-Id").unwrap();
        let second = header(&posts[1], "X-Client-Transaction-Id").unwrap();
        assert!(!tid_signs(first, "POST", path_of(&posts[0].0)), "stale key");
        assert!(tid_signs(second, "POST", path_of(&posts[1].0)));
        assert_eq!(x.gets.lock().unwrap().len(), 2, "one re-derivation");
    }

    #[tokio::test]
    async fn a_gated_404_is_resent_only_once() {
        let x = FakeX::with_graphql(&[404, 404, 404]);
        let tmp = TmpDir::new("resend-once");
        let mut ctx = test_ctx(&x, &tmp);
        let post = post_engage_graphql(&mut ctx, "CreateBookmark", "123", "k")
            .await
            .unwrap();

        assert_eq!(post.response.status, 404);
        assert!(post.resent);
        let posts = x.posts.lock().unwrap();
        assert_eq!(posts.len(), 2);
        // Derived in this run: the resend only needs a new id, not new pages.
        assert_eq!(x.gets.lock().unwrap().len(), 2);
        for p in posts.iter() {
            let tid = header(p, "X-Client-Transaction-Id").unwrap();
            assert!(tid_signs(tid, "POST", path_of(&p.0)));
        }

        let data = engage_failure_data(&ctx, "bookmark", "CreateBookmark", &post);
        assert_eq!(
            data["error"],
            "bookmark failed: HTTP 404 even with an x-client-transaction-id (resent once with a new id)"
        );
        let suggestion = data["suggestion"].as_str().unwrap();
        assert!(suggestion.contains("query id is stale"), "{suggestion}");
        assert!(suggestion.contains("doctor --refresh"), "{suggestion}");
        assert!(
            suggestion.contains("derivation has drifted"),
            "{suggestion}"
        );
    }

    #[tokio::test]
    async fn a_failed_refresh_after_a_signed_404_does_not_claim_nothing_was_sent() {
        let x = FakeX {
            home_status: 500,
            ..FakeX::with_graphql(&[404, 404])
        };
        let tmp = TmpDir::new("signed-then-refresh-fails");
        let mut ctx = test_ctx(&x, &tmp);
        load_stale_cache(&mut ctx, &tmp);
        let post = post_engage_graphql(&mut ctx, "CreateBookmark", "123", "k")
            .await
            .unwrap();

        assert!(post.first_had_tx);
        assert!(!post.resent);
        assert_eq!(x.posts.lock().unwrap().len(), 1);
        assert!(has_header(
            &x.posts.lock().unwrap()[0],
            "X-Client-Transaction-Id"
        ));
        let data = engage_failure_data(&ctx, "bookmark", "CreateBookmark", &post);
        let error = data["error"].as_str().unwrap();
        assert!(
            error.contains("404 with an x-client-transaction-id"),
            "{error}"
        );
        assert!(error.contains("home page: HTTP 500"), "{error}");
        assert!(
            !error.contains("no x-client-transaction-id was sent"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn the_resend_goes_through_the_throttle() {
        let x = FakeX::with_graphql(&[404, 200]);
        let tmp = TmpDir::new("resend-throttle");
        let mut ctx = test_ctx(&x, &tmp);
        ctx.throttle.set_endpoint(
            "k",
            twr_client::BucketConfig {
                rps: 0.001,
                burst: 1.0,
            },
        );
        post_engage_graphql(&mut ctx, "CreateBookmark", "123", "k")
            .await
            .unwrap();

        // The one token of the bucket went to the resend.
        assert!(ctx.throttle.wait_secs("k", now_f64()) > 0.0);
    }

    #[tokio::test]
    async fn a_gated_404_without_ingredients_is_not_resent_and_names_why() {
        let x = FakeX::with_graphql(&[404, 404]);
        let x = FakeX {
            home_status: 500,
            ..x
        };
        let tmp = TmpDir::new("no-ingredients");
        let mut ctx = test_ctx(&x, &tmp);
        let post = post_engage_graphql(&mut ctx, "CreateBookmark", "123", "k")
            .await
            .unwrap();

        assert_eq!(post.response.status, 404);
        assert!(!post.resent);
        assert_eq!(x.posts.lock().unwrap().len(), 1);
        let data = engage_failure_data(&ctx, "bookmark", "CreateBookmark", &post);
        assert_eq!(
            data["error"],
            "bookmark failed: HTTP 404, no x-client-transaction-id was sent (home page: HTTP 500)"
        );
        assert!(!data["error"].as_str().unwrap().contains("doctor"));
    }

    #[tokio::test]
    async fn an_ungated_404_is_neither_resent_nor_blamed_on_the_transaction_id() {
        let x = FakeX::with_graphql(&[404]);
        let tmp = TmpDir::new("ungated-404");
        let mut ctx = test_ctx(&x, &tmp);
        // Ingredients loaded from the cache must not make an ungated op retry.
        ctx.tx = Some(TxState {
            inner: twr_tx::ClientTransactionV1::new(vec![9u8; 48], "k".into()),
        });
        let post = post_engage_graphql(&mut ctx, "FavoriteTweet", "123", "k")
            .await
            .unwrap();

        assert!(!post.resent);
        assert_eq!(x.posts.lock().unwrap().len(), 1);
        assert_eq!(
            engage_failure_data(&ctx, "like", "FavoriteTweet", &post),
            serde_json::json!({"error": "like failed: HTTP 404"})
        );
    }

    fn no_instructions(_: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
        None
    }

    #[tokio::test]
    async fn both_page_fetchers_derive_and_send_the_header_for_gated_reads() {
        let x = FakeX::new(200);
        let tmp = TmpDir::new("pages");
        let mut ctx = test_ctx(&x, &tmp);
        fetch_parsed_page(
            &mut ctx,
            "SearchTimeline",
            serde_json::json!({}),
            no_instructions,
        )
        .await
        .unwrap();
        // Fresh state for the second fetcher: the first one already derived.
        ctx.tx = None;
        ctx.tx_attempted = false;
        fetch_parsed_page_generic(
            &mut ctx,
            "UserTweetsAndReplies",
            serde_json::json!({}),
            no_instructions,
            |_, _| (Vec::<u8>::new(), None),
        )
        .await
        .unwrap();

        // SearchTimeline rides POST, UserTweetsAndReplies GET: both paths.
        let gets = x.gets.lock().unwrap();
        let posts = x.posts.lock().unwrap();
        let reads: Vec<_> = gets
            .iter()
            .chain(posts.iter())
            .filter(|g| g.0.contains("/i/api/graphql/"))
            .collect();
        assert_eq!(reads.len(), 2);
        for read in reads {
            assert!(has_header(read, "X-Client-Transaction-Id"), "{}", read.0);
        }
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
