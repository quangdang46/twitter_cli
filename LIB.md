# twr as a library (issue #9)

Every workspace crate is a real library with a stable public API; the `twr`
binary's commands are thin wrappers. Rust-native consumption only — no
PyO3/FFI binding is in scope (reviewed and rejected).

## Crates

| Crate | Role | Key API |
|---|---|---|
| `twr-core` | envelope, errors, flags, safety | `Envelope::ok/err`, `emit`, `TwrError`, `decide`, `pre_check`, `Policy`, `BudgetCheck`, `TimeMode`, `apply_compact/apply_fields`, TOON |
| `twr-client` | transport + headers + throttle + timeline + upload | `HttpTransport`, `WreqTransport`, `build_headers`, `Throttle`, `fetch_timeline`, `upload_media` |
| `twr-model` | domain structs + parser | `Tweet/Author/Metrics/UserProfile`, `parse_tweet_result`, `parse_timeline_response`, `parse_article` |
| `twr-auth` | credential resolution | `resolve`, `read_env`, `extract_session_cookies`, session load/save, `LOGIN_GUIDE` |
| `twr-config` | figment resolution | `load`, `TwrConfig`, `masked_view` |
| `twr-graphql` | query IDs + features | `resolve`, `rescrape`, `compact_features`, `load_yaml` |
| `twr-tx` | transaction proof | `RequestProof`, `ClientTransactionV1`, `cache` |
| `twr-filter` | engagement scoring | `score_tweet`, `filter_tweets`, `FilterConfig` |

## Examples

```sh
cargo run --example score_tweets -p twr-filter   # offline, no creds
TWITTER_AUTH_TOKEN=… TWITTER_CT0=… cargo run --example read_timeline -p twr-client
```

## Docs

`cargo doc --workspace --no-deps` — every public item carries rustdoc.
