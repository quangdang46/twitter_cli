# COMPREHENSIVE PLAN — twitter-cli Rust Port (`twr`)

> Date: 2026-09-15. Source of truth: `tmp/_research/twitter-py` (public-clis/twitter-cli, Python 0.8.6, Apache-2.0) — cloned locally for research, not committed (see `.gitignore`).
> Reference clones studied in `tmp/_research/`: `xurl-rs`, `xmaster-cli`, `agent-x`, `xcom-rs`, `xint-rs` (official-API Rust CLIs — agent ergonomics) + `x-cli-go`, `peep`/`bird`, `agentic-x`, `xfetch`, `x-reader`, `x-cli-vibe`, `hafez-x-cli` (cookie-GraphQL scrapers — auth/queryId/anti-detect patterns) + `twitter-internal-api-doc` (reference data) + `toon_rust` (TOON output format) + our own prior `discord_cli` (agent-first Rust CLI + MCP server, proven in production).
> Evaluated & rejected: `devhindo/x` (official OAuth1 post-only + scheduler — only the `future`/schedule idea kept for a later phase), paid-API wrappers, Selenium-based tools, plain forks of the source repo.
>
> **Thesis: AGENT-FIRST, human second.** Every decision below is prioritized as: (1) machine-parseable and script-safe, (2) safe when an agent calls it wrong (policy/dry-run/idempotency), (3) only then human-readable. Human output is a thin render layer over the same data model — not the core.

---

## 0. Design philosophy — agent-first (read before implementing anything)

### 0.1 Principles

1. **Stdout is data, stderr is diagnostics.** When output is json/yaml/toon: stdout contains ONLY the envelope (exactly one document); all logs/warnings/progress go to stderr. Piping `twr search … --json | jq` must never break because a log line leaked into stdout. (Learned from xmaster-cli's pipe-safety, xurl-rs's single-print-owner + `lint-stdio.sh`, and our own discord-cli's stdout/stderr contract.)
2. **Everything has a stable exit code.** Agents retry on exit code, not by parsing messages. The table in §4.2 is a contract — frozen after P1.
3. **Every error carries a `suggestion` + `retryable`.** Agents should self-recover in ~80% of cases (re-login, wait out rate limits, refresh query IDs) without asking the user. (Learned from xmaster-cli's `suggestion()`.)
4. **Writes are safe by default:** `--dry-run` preview envelope for EVERY write; `delete/follow/unfollow` require `--apply` or confirmation; `--policy` gates at the root; idempotency key for post/reply (retry without double-posting).
5. **Self-describing:** `twr schema`, `twr commands`, `twr doctor`, `twr catalog --json` all work offline — an agent that has never read the docs can still discover the entire surface. (Learned from xcom-rs's introspection and xint-rs's `--describe/--schema`.)
6. **Deterministic when it matters:** `--no-interactive` (never prompts/hangs), `--trace-id` threading through everything, cursor-based pagination (no drifting page numbers), `--fields` projection to cut tokens.
7. **Human output is a view, not the model:** tables/colors/emoji render from the same structs; `NO_COLOR`, non-TTY auto-YAML, `--full-text` — an agent always has a path to the full data.
8. **Secrets never reach output:** cookies, `auth_token`, `ct0`, the transaction key, session file contents — MUST NOT appear in stdout, error envelopes, `doctor --json`, traces, verbose logs, or `failingInput`. `doctor` only prints `{present: true/false, source: "env|keyring|browser"}`. This is a core contract (§4.1), not just a SKILL guideline.

### 0.2 Agent interaction contract (a one-page cheat sheet, later folded into SKILL.md)

```bash
twr status --json                      # first gate: {ok, authenticated, user?}; exit 77 if not logged in
twr search "query" --json --max 20     # list -> data[] + pagination.nextCursor
twr search "query" --json --cursor "<nextCursor>"   # next page
twr tweet 123 --json                   # detail + replies
twr post "text" --json                              # no --apply -> automatic preview, network untouched (§5.3)
twr post "text" --apply --json --idempotency-key <uuid>   # confirmed + safe-to-retry write
twr post "text" --apply --policy read_only          # -> exit 2, no post sent (policy blocks it regardless of --apply)
twr schema --json / twr doctor --json  # discovery + diagnostics
```

---

## 1. Python original — exact inventory

| File | Lines | Ported into |
|---|---|---|
| `cli.py` | 1442 | `crates/twr/src/cli/` (clap derive, one module per command group) |
| `client.py` | 1172 | `crates/twr-client/` (wreq session, headers, retry, upload) |
| `graphql.py` | 227 | `crates/twr-graphql/` (query-ID resolution + bundle scraping) |
| `parser.py` | 540 | `crates/twr-model/src/parse.rs` (fixture parity tests) |
| `models.py` | 79 | `crates/twr-model/src/model.rs` (serde) |
| `auth.py` | 634 | `crates/twr-auth/` |
| `formatter.py` | 325 | `crates/twr-output/src/table.rs` (comfy-table) |
| `output.py` | 150 | `crates/twr-output/src/envelope.rs` |
| `config.py` | 161 | `crates/twr-config/` (figment) |
| `filter.py` | 95 | `crates/twr-filter/` |
| `search.py` | 128 | folded into `cli/search.rs` + the client's query builder |
| `serialization.py` | 244 | `twr-model/src/serde.rs` (`--input/--output` round-trip) |
| `timeutil.py` | 82 | `chrono` + small helpers |
| `cache.py` | 65 | `show N` last-list cache (`~/.twr/last.json`) + query-ID cache |
| `constants.py` | 121 | `twr-graphql/src/consts.rs` + `twr-client/src/headers.rs` |
| `exceptions.py` | 78 | `twr-error` (thiserror + exit codes) |

### 1.1 Command parity matrix (100% *source command coverage* by Phase 1–3 — see note below)

> "Parity" here means every command below exists with equivalent read/write capability — it does NOT mean identical behavior in every edge case. `twr` intentionally diverges from the Python original wherever the agent contract demands it: the `--apply`/`--dry-run` write-safety model (§5.3), `--policy` gating, exit codes, idempotency semantics, and configurable (not hardcoded) limits are deliberate, documented behavioral changes, not parity gaps to close.

Read: `feed [-t for-you|following]`, `bookmarks [folders [folder_id] [--since]]`, `search [QUERY]`, `tweet ID|URL`, `show INDEX`, `article ID|URL [--markdown]`, `list LIST_ID`, `user H`, `user-posts H`, `likes H`, `followers H`, `following H`, `status`, `whoami`.
Write: `post TEXT [--reply-to] [-i ×4]`, `reply ID TEXT [-i]`, `quote ID TEXT [-i]`, `delete ID` (confirm), `like/unlike`, `retweet/unretweet`, `favorite/bookmark/unfavorite/unbookmark` (aliases preserved), `follow/unfollow`.
Global flags on every read command: `-n/--max`, `--cursor` (feed/list), `--input/-i`, `--output/-o`, `--filter`, `--full-text`, `--json/--yaml`; search adds `-t Top|Latest|Photos|Videos`, `--from/--to/--lang/--since/--until`, `--has` (repeatable), `--exclude` (repeatable), `--min-likes/--min-retweets`; root: `-v/--verbose`, `-c/--compact`.
Write commands: `--json/--yaml`.

### 1.2 Constants that must be ported verbatim

- **Bearer token** (`constants.py`): `AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA`. Note: this is X's own **public web-client** bearer, distinct in security class from a user's `auth_token`/`ct0`/transaction key — treat it as a replaceable constant, never as a user credential.
- **22 FALLBACK_QUERY_IDS** (`graphql.py`): HomeTimeline `c-CzHF1LboFilMpsx4ZCrQ`, HomeLatestTimeline `BKB7oi212Fi7kQtCBGE4zA`, UserByScreenName `1VOOyvKkiI3FMmkeDNxM9A`, UserTweets `q6xj5bs0hapm9309hexA_g`, TweetDetail `xd_EMdYvB9hfZsZ6Idri0w`, Likes `lIDpu_NWL7_VhimGGt0o6A`, SearchTimeline `VhUd6vHVmLBcw0uX-6jMLA`, Bookmarks `2neUNDqrrFzbLui8yallcQ`, ListLatestTweetsTimeline `RlZzktZY_9wJynoepm8ZsA`, Followers `IOh4aS6UdGWGJUYTqliQ7Q`, Following `zx6e-TLzRkeDO_a7p4b3JQ`, CreateTweet `IID9x6WsdMnTlXnzXGq8ng`, DeleteTweet `VaenaVgh5q5ih7kvyVjgtg`, FavoriteTweet `lI07N6Otwv1PhnEgXILM7A`, UnfavoriteTweet `ZYKSe-w7KEslx3JhSIk5LA`, CreateRetweet `ojPdsZsimiJrUGLR1sjUtA`, DeleteRetweet `iQtK4dl5hBmXewYZuEOKVw`, CreateBookmark `aoDbu3RHznuiSkQ9aNM67Q`, DeleteBookmark `Wlmlj2-xzyS1GN3a6cj-mQ`, TweetResultByRestId `7xflPyRiUxGVbJd4uWmbfg`, BookmarkFoldersSlice `i78YDd0Tza-dV4SYs58kRg`, BookmarkFolderTimeline `hNY7X2xE2N7HVF6Qb_mu6w`.
- **FEATURES** — 21 flags plus per-call extras (fetch_user adds hidden/tipjar/subscriptions/highlights/article-notes/gift toggles; fetch_article adds `articles_preview_enabled` + rich-content toggles; tweet_detail sets `withArticleRichContentState=True`). Strip false-valued features to avoid HTTP 414.
- **Headers** (`client.py::_build_headers`): Bearer, Cookie (full string or `auth_token;ct0`), `X-Csrf-Token=ct0`, `X-Twitter-Active-User: yes`, `X-Twitter-Auth-Type: OAuth2Session`, `X-Twitter-Client-Language`, a Chrome/{133+} UA matched to the OS, Origin/Referer, dynamic `sec-ch-ua*` (arch/platform), `Sec-Fetch-*`, POST JSON + `Priority: u=1,i`, `X-Client-Transaction-Id`. Upload endpoint `upload.twitter.com/i/media/upload.json`; follow endpoints `1.1/friendships/create|destroy.json`.
- **Endpoints/pagination**: `_fetch_timeline` (id dedup, jittered `requestDelay`, `count=min(remaining+5,40)`); search/followers/following use POST; `fetch_me` falls back through `multi/list.json` → GraphQL.
- **Upload**: INIT→APPEND→FINALIZE, ≤5MB (jpeg/png/gif/webp); **+ PR #41**: chunked GIF up to 15MB (`tweet_gif`, 1MB chunks), raw-binary multipart (drops base64 encoding), `--compress N` (via the `image` crate, optional dependency).
- **Env vars**: `TWITTER_AUTH_TOKEN`+`TWITTER_CT0`, `TWITTER_BROWSER`, `TWITTER_CHROME_PROFILE`, `TWITTER_PROXY`, `OUTPUT`, locale (`LC_ALL`/`LC_MESSAGES`/`LANG`).
- **Errors**: `not_authenticated` (401/403) / `rate_limited` (429, inner codes 88/348/349) / `not_found` (404) / `invalid_input` / `network_error` / `query_id_error` / `media_upload_error` / `api_error`. SCHEMA envelope `ok/schema_version:"1"/data|error`, non-TTY defaults to YAML.
- **Config** (`config.yaml`): `fetch.count` 50; filter `topN`/20/50 with weights `{likes:1.0, retweets:3.0, replies:2.0, bookmarks:5.0, views_log:0.5}`; `rateLimit` `{1.5, retries:3, backoff:5.0, maxCount:200}`. Score = likes + 3·retweets + 2·replies + 5·bookmarks + 0.5·log10(views).
- **Parser edge cases** (must have fixture tests): tombstone/visibility unwrapping, retweet unwrapping, `note_tweet` full text, media (photo-original / best video mp4), quote tweets, article Draft.js→Markdown conversion, the promoted flag.

### 1.3 Upstream PRs

- **#41 (closed, never merged)**: port directly into Phase 2 (§1.2 Upload).
- **#31 (open, +2736/-67)**: an official-API-v2 backend running alongside cookie auth — deferred to **Phase 4**, feature-gated as `--backend api-v2` (Bearer/OAuth2, video upload, `--video/--file/--alt-text`, `search --scope`, `--reply-scope`). Phases 1–3 ship cookie-backend only.

---

## 2. Lessons from 15 reference repos

### 2.1 Cookie-GraphQL CLIs
- **x-cli-vibe (Go)** — the strongest transport-compatibility story: uTLS `HelloChrome_120`, per-endpoint token bucket driven by a data file (`endpoints.yaml`: rps/burst), mutation throttling (min 8s/max 22s jitter, daily cap 200, auto-pause), dry-run by default + explicit `--apply`, OS keychain with an AES-GCM file fallback. **Steal**: data-driven `endpoints.yaml`, the throttle design, the `--apply` gate.
- **agentic-x (Python)** — a **real** transaction ID (a port of `XClientTransaction`; single-use; only attached to GATED_OPS = {SearchTimeline, UserTweetsAndReplies, Followers}); browser-free re-anchoring of query IDs (walks `main.js` plus up to 800 lazy chunks); exit code 4 = contract drift. **Steal**: the full transaction-derivation logic, the gated-ops rule, `doctor --refresh`.
- **x-cli-go (Go)** — three access tiers (syndication/guest/session), guest tokens minted via `api.x.com/1.1/guest/activate.json`, exit codes 2–8, `-o table|csv|tsv|url|json|jsonl|raw|template`, best-effort transaction IDs (omitted, never fatal, when unavailable). **Steal**: the tier model for degraded-mode operation, the exit-code table.
- **peep/bird + x-reader (TS)** — a baseline `query-ids.json` with a 24h cache plus an `EXTRA_QUERY_ID_FALLBACKS` rotation list tried on 404, with a force-refresh retry loop; `sweet-cookie` for cookie extraction; a `--allow-write` gate. **Warning**: their transaction ID is a stub (random hex) — this is WRONG, do not copy it. **Steal**: the fallback-rotation + refresh-retry loop.
- **xfetch** — a session pool, a `RateLimiter` driven by response headers, proxy rotation, resumable pagination. **Steal**: the pool design + header-based rate tracking.
- **hafez-x-cli** — a compact guest-token flow with thread context; useful reference for the guest path.
- **twitter-internal-api-doc** — 104 operations (query ID + features/toggles), but IDs rot every 2–4 weeks → use as build-time inventory only (feed into `build.rs` + re-scrape in CI, never trust a committed ID).

### 2.2 Official-API Rust CLIs — the agent-surface recipe
Combine: the envelope model from **xurl-rs**, `type`/`schemaVersion`/`traceId` from **xcom-rs**, auto-JSON-when-piped and `suggestion()` from **xmaster-cli**, `--policy`/`--fields` from **xint-rs**, the `Renderable` trait from **agent-x**, plus idempotency (xcom-rs), introspection commands, cost guards, and `--dry-run`/`--force`/`--no-interactive`.

### 2.3 `xf` (Dicklesworthstone, v0.4.1) — agent-first patterns worth stealing (studied 2026-09-15)

> Different domain (offline X-archive search: Tantivy BM25 + SQLite + embeddings), same agent-first philosophy. Single-crate (~1MB `src/`, 171KB `main.rs` — do NOT copy that layout; `twr`'s workspace split stays). Take the CLI-surface patterns, leave the search engine.
- **Global flags, agent-shaped** (`xf/src/cli.rs`): `--db/--index` with `env=` fallback + `global=true`, `--format/-f` with precedence `XF_OUTPUT_FORMAT > TOON_DEFAULT_FORMAT`, `-v` counted verbosity + `-q` quiet (quiet wins), `--no-color` honoring `NO_COLOR`. **Steal**: this exact bundle for `twr`'s root flags, plus `Verbosity::from_flags(quiet, verbose_count)` → tracing filter mapping (`output.rs`, `logging.rs`). Partially landed via 3.3.8 (`-v` bool, `--compact/--fields`); still open in 3.3.14: `env=` fallbacks, counted `-vv`, `-q`, `--no-color`.
- **`robot-docs` → grow `twr schema`** (`xf/src/robot_docs.rs`): `xf robot-docs <topic>` returns versioned JSON (`SCHEMA_VERSION` const, `TOPICS` list, `is_valid_topic()`) covering commands/flags/examples/exit-codes/output-formats/schemas. **Steal**: grow `twr schema` into `twr schema [topic]` with the same shape instead of adding a new subcommand (3.3.9 shipped schema/commands catalogs without this — follow-up).
- **`doctor` result shape** (`xf/src/doctor.rs`): `HealthCheck{category, name, status: Pass|Warning|Error, message, suggestion?}` with `is_ok()` true only on `Pass`. **Steal**: reuse verbatim for `twr doctor`; categories become `auth/query-id/features/tx-id/response-shape/tls` (3.3.9 shipped 5 checks without this shape — follow-up, keeping the RESPONSE_SHAPE-vs-QUERY_ID distinction).
- **Progress auto-disable** (`xf/src/progress.rs`): indicatif bars/spinners that self-disable on non-TTY or non-text formats — the executable form of our §0.1 #1 stdout/stderr rule. Copy at P3 polish / when read commands land (3.3.12).
- **`install.sh` harder than ours** (390 vs 283 lines): SHA256 verify (`--checksum/--checksum-url/--insecure-skip-checksum`), `--verify` self-test post-install, `/tmp` lock file, cache-buster one-liner. Ours (5.5.10, closed) already covers checksum/verify/lock — only gaps are cache-buster one-liner + `--checksum-url`, adopt on need.
- **Cheap wins**: `completions.rs` pipes scripts to stdout / instructions to stderr (5.5.5); `date_parser.rs` (`chrono-english` + month/year-range helpers for `--since/--until` in 3.3.12); `canonicalize.rs` pipeline (NFC → strip markdown → collapse code blocks → drop low-signal → truncate) if embeddings ever land.
- **Do NOT take**: single-crate layout, daemon + MessagePack transport, frankensearch/ort/fastembed dependency weight, `opt-level="z" + panic="abort"` release profile (distribution tuning, not dev-phase).
- **`twr` stays ahead where it matters**: frozen exit-code table with payload-shape 404 disambiguation (§5.2) and constructor-enforced secret redaction (§5.3) — `xf` has neither (plain thiserror + scattered `exit(1/2)`). No regression here.

### 2.4 Our own `discord_cli` — proven patterns, ported directly (not "borrowed", *reused*)
We already shipped an agent-first Rust CLI (user-account Discord client + MCP server, 77 commands). Its patterns transfer almost unchanged:
- **Envelope + exit codes**: `{ok, schema_version:"1", data|error}`; stdout=data (JSONL when piped, single envelope with `--json`), stderr=diagnostics; exit 0 ok / 2 usage (missing `--confirm`) / 3 not-found / 4 forbidden/rate-limited. `twr` extends this table (§4.2) with drift (6) and file-IO (7) codes specific to a scraping backend.
- **`--confirm` gating + exact denial message**: `This will <action> "<target>". Add --confirm to proceed.` — copied verbatim as the pattern for `twr`'s `--apply`/`--force`.
- **`--transcript` compact mode**: a plain-text rendering ~5x smaller than JSON, built for AI summarization — the direct ancestor of `--compact`/`--fields` here.
- **Token resolution order + OS keychain storage**: flag → env → `.env` → OS keyring, no interactive prompts unless explicitly asked for — the template for `twr-auth`'s resolution chain (adjusted for browser cookie extraction instead of a single bot/user token).
- **SQLite + FTS5 offline archive** (WAL mode, upsert-never-replace to preserve foreign keys, cursor-based sync state): directly portable to a local tweet/timeline cache in a later phase.
- **`discord serve` MCP server** (rmcp, stdio transport, ~40 tools, JSON-string returns): the template for `twr mcp` in the extras phase.
- **Release pipeline**: `install.sh`/`install.ps1` with checksum verification, atomic install, PATH auto-update; a 3-OS release matrix (linux musl x86_64/aarch64, macOS x86_64/aarch64, windows x86_64) — reused as-is for `twr`'s distribution.

---

## 3. Locked-in crate choices

| Role | Crate | Notes |
|---|---|---|
| Transport (abstracted!) | `trait HttpTransport` → `WreqTransport` (wreq 0.15+ crates.io-resolved for this toolchain, `wreq_util::Emulation::Chrome131` (Emulation lives in the separate wreq-util crate, not wreq itself -- confirmed while implementing P0-1)) / `CurlImpersonateTransport` (curl-impersonate-cli subprocess) | Never plain reqwest (its TLS fingerprint gives it away). All call sites go through the trait — wreq is the default implementation, not an architectural dependency. Vocabulary: "browser-compatible transport fingerprinting", not "anti-detection". |
| CLI | `clap` 4 + `clap_complete` 4 | derive macros; every flag also takes `env=TWITTER_*` |
| Table | `comfy-table` 7 + `crossterm` | Windows UTF-8 fix, mirroring `formatter.py` |
| Output | `serde_json` 1 + `serde_yaml` 0.9 | envelope stays SCHEMA-compatible with the Python original |
| TOON output | `toon` (Dicklesworthstone/toon_rust; `json_to_toon`/`toon_to_json`) | `--toon` renders the same envelope as token-efficient tabular text for bulk reads (`tweets[N]{id,text,screen_name,likes,…}`); a renderer, not a different data model. `--input *.toon` is a valid decode path. Ships in a later phase once the core envelope/exit contract is frozen. |
| Config | `figment` 0.10 (toml + env) | mirrors xmaster-cli's `XMASTER_KEYS__*` env-nesting pattern |
| Cookies | `rookie` + `cookie_store` | Chrome/Edge/Arc/Firefox/Brave extraction; validated in the P0 spike; always has a manual full-cookie-string fallback |
| Keychain | `keyring` | session persistence, following x-cli-vibe's design |
| Async/retry | `tokio` 1 (full) + `backoff` + `rand` | jitter and exponential backoff on 429 |
| Media | `image` (optional, for `--compress`) + `mime_guess` | PR #41 |
| Time/IDs | `chrono` + `uuid` v4 | trace IDs, `--since`/`--until` parsing |
| Errors | `thiserror` 2 + `anyhow` 1 | |
| SQLite (later phase) | `rusqlite` (bundled) 0.31+ | local cache + watchlist, following our discord-cli's schema |
| Transaction proof | `sha2`, `base64` | implements the `RequestProof` trait, porting agentic-x's `transaction.py` |

---

## 4. Target architecture

```
twitter-cli-rs/  (binary `twr`, `twitter` as an opt-in alias)
├── crates/
│   ├── twr/               # main.rs, cli/ (read|write|search|user|list|article|auth|debug)
│   ├── twr-client/        # session, headers.rs, timeline.rs, write.rs, upload.rs, v2.rs (later phase, feature api-v2)
│   ├── twr-graphql/       # consts.rs, resolve.rs, scrape.rs, endpoints.yaml (data-driven)
│   ├── twr-tx/            # RequestProof trait + ClientTransactionV1 impl; single-use, GATED_OPS-only
│   ├── twr-auth/          # env -> keyring/file -> browser extraction -> verify; --cookie-source/--chrome-profile
│   ├── twr-model/         # model.rs + parse.rs + serde.rs (fixture parity)
│   ├── twr-filter/        # topN|score|all scoring
│   ├── twr-output/        # envelope.rs + table.rs + error suggestions
│   ├── twr-config/        # figment: config.yaml + TWITTER_* env
│   └── twr-error/         # kind() (kebab-case) + exit codes + suggestion + is_retryable
├── SKILL.md + SCHEMA.md
├── endpoints.yaml
└── tests/fixtures/        # JSON captures from `twitter --json` (the Python original)
```

---

## 5. AGENT CONTRACT — detailed spec (the core of this plan)

### 5.1 Envelope — a protocol invariant (non-negotiable)

```
stdout: exactly one machine-readable document (the envelope), never mixed with logs/warnings
stderr: diagnostics only (verbose logs, progress, per-page cursor under -v)
```

Every warning (e.g. a ClientTransaction init failure — upstream issue #69 once broke JSON output this exact way) MUST go to stderr. Test: `twr <any> --json | python -c json.load` always passes.

> `--toon`: TOON is a **renderer** (Domain data → Envelope → Renderer: JSON/YAML/TOON), not a JSON-then-convert step. Tabular arrays for lists (`tweets[N]{id,text,…}`). Token savings are workload-dependent — benchmark representative payloads (a 50-tweet feed/search) before quoting a number; no fixed percentage belongs in the contract. Errors always stay JSON/YAML, never TOON.

Success:
```json
{"ok":true,"schema_version":"1","type":"tweet_list","data":[…],"pagination":{"nextCursor":"…","hasMore":true},
 "meta":{"traceId":"…","command":"search","maxRequested":20,"returned":20,"filterApplied":false}}
```
Error (always on stdout — one stream for the agent to read; debug logs go to stderr):
```json
{"ok":false,"schema_version":"1","type":"error","error":{"code":"rate_limited","message":"…",
 "suggestion":"Wait ~15 min or set TWITTER_PROXY, then retry with --cursor …","retryable":true,"retryAfterMs":900000,
 "failingInput":"--max 500"},"meta":{"traceId":"…"}}
```
- `type`: `tweet_list|user_list|user|tweet_detail|article|status|auth|write_result|schema|commands|doctor|dry_run`.
- `--compact/-c`: strips `author.profile_image_url`, `media[].width/height`, expanded `urls` → keeps `id/text/author.screen_name/metrics/created_at`.
- `--fields a.b,c`: post-parse projection (e.g. `--fields id,text,author.screen_name,metrics.likes`).
- `--input/-i` + `--output/-o`: file round-trip, envelope preserved (parity with `serialization.py`).
- `show N`: reads `~/.twr/last.json` (written after every list command) — an agent never has to copy-paste an ID.

### 5.2 Exit codes (contract frozen after P1)

| Exit | Meaning | Agent action |
|---|---|---|
| 0 | success (including dry-run) | parse `data` |
| 1 | general/auth-resolution failure (config error) | run `twr status`, fix per `suggestion`; do not blind-retry |
| 2 | usage/policy-denied (missing `--confirm`/`--apply`, bad flags, policy blocked) | fix args or change policy; message format `This will <action> "<target>". Add --confirm to proceed.` (from our discord-cli's `check_confirm()`) |
| 3 | not-found (the *target* genuinely doesn't exist: user/tweet/list ID is real X data that's missing/deleted/suspended) | verify the ID/handle, do not retry |
| 4 | forbidden/rate-limited (429/88/348/349, 226 automated-behavior) | back off `retryAfterMs` then resume with `--cursor`; partial data ships with `meta.truncated=true` |
| 5 | network/timeout | exponential-backoff retry |
| 6 | contract drift (a GraphQL 404 caused by a *stale query ID*, not a missing target; feature/toggle rejection; response-shape/parser mismatch; envelope-parse failure) | `twr doctor --refresh` once, then retry once (doctor classifies which layer drifted, §5.4) |
| 7 | attachment/file-IO (`--input`/`--output`/`-i` media errors) | fix the path/permissions/size |
| 77 | auth required (401/403, no cookies) | `twr status` → guide login, do not blind-retry |

**Disambiguating the two "404"s (exit 3 vs. exit 6):** GraphQL surfaces two unrelated failures under similar transport codes — an HTTP 404 on the endpoint itself (the query ID is stale/invalid → exit 6, contract drift) vs. a successful HTTP 200 whose payload says the *target* doesn't exist (a `TweetUnavailable`/`UserUnavailable`/tombstone result → exit 3, not-found). `twr-error`'s mapping (§7) must classify on payload shape, never on HTTP status alone — see the `twr-error` unit tests.

### 5.3 Root flags (every command)

```
--json/--yaml/--toon     # machine output (auto-YAML when piped without a flag, matching the Python original; --toon = token-efficient, §5.1)
--compact/-c             # minimal fields
--fields <csv>           # projection
--policy <read_only|engagement|write>   # default write; read_only blocks all writes; engagement allows like/rt/follow/bookmark but blocks post/delete
--dry-run                # explicit preview: return a DryRun envelope {dry_run:true, operation, validation:"passed"} — never "would_succeed" (no guarantee a later real call succeeds)
--force/--apply          # REQUIRED to actually execute any write (post/reply/quote/delete/like/retweet/bookmark/follow/…) — this is the "safe by default" mechanism (§0.1 #4), not just a convention for delete/follow
--no-interactive         # never prompt. See the write-command decision table right below — a write with neither --apply nor --dry-run under --no-interactive is a usage error, not a silent action
--trace-id <id>          # threaded through the whole call (auto-generated uuid if omitted)
--timeout <s> --max-retries <n>         # override config
-v/--verbose             # diagnostics to STDERR (never pollutes stdout)
```

**Write-command decision table** — resolves what happens when a write is invoked with various combinations of `--apply`/`--dry-run`/`--no-interactive`:

| `--apply` | `--dry-run` | `--no-interactive` | Behavior |
|---|---|---|---|
| no | no | no (TTY) | prompt for confirmation; on yes → execute, on no/EOF → exit 0 with a `dry_run`-shaped cancellation note |
| no | no | yes | **exit 2** (usage/policy-denied) — the call is ambiguous by design; an agent must say which it wants |
| no | yes | either | run the DryRun preview, exit 0, never touch the network |
| yes | no | either | execute for real |
| yes | yes | either | **exit 2** — `--apply` and `--dry-run` are mutually exclusive, this is a usage error |

`failingInput` in error envelopes MUST be redacted before serialization: flags whose values are secrets (`--cookie`, `--auth-token`, `--ct0`, `--proxy` when it embeds credentials) are reported as `{"flag": "--cookie", "value": "[REDACTED]"}`, never the raw value. This is a core contract (§0.1 #8), enforced in `twr-error` construction, not left to call sites.

### 5.4 Introspection (offline, no auth required)

- `twr schema [--json]` — JSON Schema for every envelope `type` (following xurl-rs's `xr schema` and xcom-rs).
- `twr commands [--json]` — a catalog: name, args, flags, policy level, idempotent?, examples. An agent can build a correct call without reading docs.
- `twr doctor [--refresh] [--json]` — independent checks, each `{check, status, suggestion}`: `QUERY_ID` (baseline vs. live, which op is stale) / `FEATURES` / `TOGGLES` / `TX_ID` (fresh key? gated-op probe) / `AUTH` (session alive?) / `RESPONSE_SHAPE` (a lightweight parse probe) / `TLS` (transport reachability). A correct query ID with a failing parser is a `RESPONSE_SHAPE` failure — `--refresh`-ing the query ID will not fix it, and doctor must say so explicitly. `--refresh` re-anchors query IDs and the transaction key.
- `twr status [--json]` — the auth gate: `{authenticated, user?, cookieSource}`; exit 77 if not authenticated.
- `twr query-ids [--fresh] [--json]` — inspect/refresh the cache (following bird/peep).
- `twr catalog` — commands + schema combined, for an MCP dump (`--describe`, following xint-rs).
- Later phase: `twr mcp` (a stdio MCP bridge, following xurl) so an agent calls tools instead of a subprocess.

### 5.5 Idempotency & write safety

- `post/reply/quote` accept `--idempotency-key` (stored in `~/.twr/idempotency.json` for 24h): this is **client-side, best-effort retry deduplication — NOT server-side exactly-once**. X's unofficial GraphQL has no native idempotency contract like Stripe's. States: `prepared|sent|acknowledged|unknown`. If `unknown` (a timeout after sending, with no way to know whether the server actually posted): **never auto-retry** — return an envelope reporting `unknown` with a suggestion to check manually. The same key in the `acknowledged` state returns the cached result instead of resubmitting — this dedups only within this local state, it does not prove the server saw exactly one request.
- Write delay of 1.5–4s with jitter between pages (preserving the Python behavior) — an agent cannot tune this below the floor, only raise it.
- A daily mutation budget, default 200 (following x-cli-vibe) — this is a **local safety policy**, independent of and not implying anything about X's actual server-side rate limit; exceeding it exits 2 with a suggestion. Configurable upward, never disabled entirely.
- `delete` always prints a preview (text prefix + ID) before running, even with `--apply` — see the write-command decision table in §5.3 for the general `--apply`/`--dry-run`/`--no-interactive` rules that now apply uniformly to every write, not just `delete`/`follow`.

### 5.6 Pagination contract for agents

- Every list returns `pagination.{nextCursor,hasMore}`. `--cursor ""` means the first page. `--max N` is the total item count desired — the client loops pages internally, dedups, and respects a *configurable safety limit* (default 200, raisable via config/flag up to a bounded ceiling; see §10's note on issue #50 — there is no `maxCount` literal of 200/500 baked into the contract, only "the currently configured limit").
- If rate-limited mid-fetch: return partial `data` + `pagination.nextCursor` + `meta.truncated=true` + exit 4 → the agent resumes from the cursor without losing data already fetched.
- `--all` always requires one of `--max-pages K` / `--max N` / an explicit `--unbounded` — no infinite agent loops are allowed by default.

---

## 6. Per-command agent spec (input → output type → command-specific errors)

| Command | Output `type` | Agent notes |
|---|---|---|
| `feed [-t]` | `tweet_list` | `for-you` = algorithmic; `following` = chronological. `--cursor` resumes |
| `bookmarks [folders [id] [--since]]` | `tweet_list` / `bookmark_folder_list` | folder timeline filters client-side by `--since` |
| `search` | `tweet_list` | query builder: `--from/--to/--lang/--since/--until/--has/--exclude/--min-likes/--min-retweets`; `-t Latest` requires a transaction ID (a GATED op) |
| `tweet ID\|URL` | `tweet_detail` {tweet, replies[], nextCursor} | accepts a URL directly |
| `show N` | `tweet_detail` | indexes into `last.json`; out-of-range → exit 3 (not-found, the index doesn't resolve to anything) + `failingInput` |
| `article` | `article` {articleTitle, articleText(md)} | `--markdown` prints raw markdown; `--output file` |
| `list ID` | `tweet_list` | cursor support |
| `user/user-posts/likes/followers/following` | `user` / `tweet_list` / `user_list` | likes are own-account-only (noted in the schema) |
| `status/whoami` | `status` / `user` | the gate for every agent workflow |
| `post/reply/quote` | `write_result` {id, url} / `dry_run` without `--apply` | policy-gated; up to 4 media (`-i` repeated), `--compress`; needs `--apply` to actually post (§5.3 decision table) |
| `delete/like/unlike/retweet/unretweet/bookmark/follow/unfollow` | `write_result` / `dry_run` without `--apply` | policy-gated; every one of these now needs `--apply` uniformly (§5.3), not just `delete`/`follow` |

---

## 7. Module implementation spec (what each crate does)

- **twr-error**: `TwrError{kind: ErrorKind(kebab-case), message, suggestion, retryable, retry_after, failing_input}`; `failing_input` is constructed pre-redacted (§5.3) — never built from raw argv; `exit_code()` per §5.2; `is_retryable()` (rate/network/5xx); unit tests mapping status → kind: 401/403→auth-required, 429/inner 88/348/349→forbidden/rate-limited, an HTTP 404 on the GraphQL endpoint itself→contract-drift, a 200 response whose payload is a tombstone/`*Unavailable` result→not-found (see §5.2's disambiguation note — classify on payload, not transport status), 226→automated-behavior + fallback to the legacy `statuses/update.json` endpoint (following bird).
- **twr-graphql**: four-layer resolution `shipped baseline → 24h disk cache (~/.twr/query-ids.json) → EXTRA rotation list → live rescrape`; `scrape.rs` walks bundles two levels deep (`main.js` → up to 800 lazy chunks, 16 workers, a strict `queryId/operationName/operationType` regex, following agentic-x); a 404 invalidates that op, triggers one refresh, then one retry; `TWR_QID_<OP>` env vars pin an operation manually; `endpoints.yaml` holds per-op `{queryId, features, toggles, rps, burst}`, read at runtime with no rebuild needed.
- **twr-tx** (interface `trait RequestProof { fn prepare(&self, op: Operation) -> Option<Proof> }`, current impl `ClientTransactionV1` porting `agentic-x/transaction.py`): fetch the home page → verification metadata + the four loading-animation SVG frames + on-demand indices → key derivation; cached for 1h (`transaction_cache.json`); a fresh ID on EVERY request; attached ONLY to GATED_OPS; unit tests check parity against the Python vectors. If X changes or drops the requirement, swap the implementation — call sites never change.
- **twr-client**: `wreq` + `wreq_util::Emulation::Chrome131` + dynamic UA/sec-ch-ua matching; a timeline loop (id dedup, `count=min(remaining+5,40)`, jitter, partial-resume); search/followers/following use POST; a `fetch_me` fallback chain; exponential backoff on 429 (5s base × 3 retries); `TWITTER_PROXY` support; a per-endpoint token bucket driven by `endpoints.yaml`.
- **twr-auth**: resolution order `flags → env → keyring/file (~/.twr/session) → browser extraction (rookie, ordered by TWITTER_BROWSER, TWITTER_CHROME_PROFILE)`; verification via `verify_credentials` → `settings.json`; a 401/403 triggers one re-extraction attempt before failing with exit 77; `login`/`logout` commands (login refuses to overwrite a still-valid session, following agent-x); the full-cookie-string paste fallback (Method C).
- **twr-model**: serde structs (Tweet/Author/Metrics/Media/UserProfile/BookmarkFolder — field names preserved from Python for fixture compatibility); `parse.rs` handles the edge cases from §1.2; parity tests assert deep-equality between each `tests/fixtures/*.json` capture and the Rust output.
- **twr-output**: `envelope.rs` (§5.1) with a single print owner (stdout written exactly once); `table.rs` via comfy-table (#/Author/Tweet/Stats/Score columns, 120-char truncation unless `--full-text`, Windows UTF-8 fix); non-TTY auto-YAML unless overridden; `NO_COLOR` respected.
- **twr-config**: figment resolution `cwd config.yaml → ~/.twr/config.yaml → defaults`; deep-merge + normalization; env override via `TWITTER_*`; `twr config show` masks secrets.
- **twr-filter**: the scoring formula preserved exactly; `--filter` is opt-in, off by default (matching Python).

## 8. SKILL.md outline (written once the core contract stabilizes, validated against a real agent)

1. Gate: `twr status --json` before anything else; exit 77 → guide login (browser extraction / env / cookie paste), never echo secrets.
2. Reading: search/feed/tweet/show/article/list/user*, always start with `--json --max 20`, use the cursor to page further, `--compact` when context is tight.
3. Writing: without `--apply` every write is already a safe preview (§5.3) — an agent should call it plain first, show the user the preview, then re-run with `--apply --idempotency-key <uuid>` once confirmed; writes need full browser cookies (env-only auth risks a 226).
4. Errors: the exit→action table (§5.2); wait `retryAfterMs` on a 4; on a 6, run `doctor --refresh` once.
5. Safety: `--policy read_only` for read-only tasks; no bulk operations; note on proxy usage.

## 9. Phases + acceptance criteria

- **P0 — Viability spike (1–2 days)**: wreq's request success rate against x.com; `rookie` extraction on Windows/Linux/macOS; a full port of the transaction-derivation logic plus one end-to-end `UserByScreenName` call. This is a go/no-go gate — if it fails, the 15-module port does not start.
- **P1 — Read MVP + agent contract**: auth resolution + verification, the four-layer query-ID resolver, the parser plus 20+ fixture parity tests, the full agent contract (envelope/exit codes/root flags/introspection), all read commands. Acceptance: `cargo test` green; `--json` output matches the Python original both byte-for-byte on fixtures (`id`/`text`/`author`/`metrics`/`media`/pagination fields — deep-equal) *and* semantically (representative multi-page cursor walks resume to the same set of tweets as Python does, not just single-page snapshot equality); `doctor`/`schema`/`commands` all pass offline.
- **P2 — Writes + media + safety hardening**: all write commands plus delays, media upload (raw-binary + chunked GIF + `--compress`), idempotency, policy gating, the `--apply`/`--dry-run`/`--no-interactive` decision table (§5.3). Acceptance, stated as the state machine actually guarantees (§5.5) rather than an exactly-once claim: an `acknowledged` idempotency key returns the cached result instead of resubmitting (no duplicate client-initiated POST is made); an `unknown`-state key never auto-retries; `read_only` policy blocks every write with exit 2; every write without `--apply` produces a `dry_run` envelope and touches the network zero times.
- **P3 — Human polish + SKILL**: tables, article-to-Markdown, filter scoring, shell completions, `SKILL.md`/`SCHEMA.md`, 3-OS CI, curl-installable release (install.sh/ps1).
- **P4 — Official API v2** (per PR #31, feature-gated as `--backend api-v2`): OAuth2 user-context, dual-backend routing per command.
- **P5 — Extras**: SQLite cache/watchlist/expanded `doctor`, a `future`/schedule command, guest-tier reads, `twr mcp`.

## 10. Issue coverage — every open issue in the source repo, accounted for

| # | Issue | Upstream status | Fix in the Rust port |
|---|---|---|---|
| #50 | Pagination capped at 500 (`_ABSOLUTE_MAX_COUNT`) + a config-path bug (`parent.parent` resolves into site-packages) | open | (a) The hard 500 ceiling is removed but replaced with a *configurable* safety limit (default 200, raisable through config/flags up to a bounded maximum) — never truly unbounded, since an agent-facing tool must not allow `--max 200000` by accident. (b) Config lookup is `cwd config.yaml → ~/.twr/config.yaml → defaults` (figment), never resolved relative to the install directory — covered by a regression test asserting the path. P1 |
| #47 | "Today's News" headlines (X's Search Navigation surface) | open, enhancement | A new `twr headlines [--json]` command (later phase): reads the Search Navigation / Today's News surface via session GraphQL; output type `headline_list`. If X doesn't expose a stable GraphQL path, falls back to a trending search by `min_faves`. Deferred because the endpoint isn't yet verified. |
| #46 | Cookie retrieval fails (401 after `uv tool install`, unclear where to set env vars) | open | `twr status` is a diagnostic gate that distinguishes "no cookies" vs. "expired (401/403)" vs. "partial (env-only → 226 risk)"; `twr login --guide` prints all three auth methods (browser/env/paste) with doc links; a first-run wizard triggers when no session exists. P1 |
| #45 | Where is the transaction-ID algorithm implemented? (closed, transparency complaint) | closed | `twr-tx` is its own crate with the algorithm documented in-repo, plus `twr doctor --explain-tx` printing the derivation steps — not a black box behind an external library the way the Python original depends on `xclienttransaction`. P0 |
| #36 | Shell completions (bash/zsh/fish) | open | `clap_complete`: `twr completions <bash\|zsh\|fish\|powershell\|elvish>`, documented in SKILL.md and the P3 install guide. |
| #35 | Absolute time display (default is relative, e.g. "28s ago") | open | A global `--time <relative\|absolute\|both>` flag (default relative, matching Python parity); absolute mode is ISO 8601 with local offset. Machine output (json/yaml/toon) ALWAYS carries an absolute `created_at` regardless of the flag — only the human table is affected. |
| #28 | Windows: browser cookie extraction fails (DB locked while Chrome runs, AES-GCM decryption fails on Chrome ≥v127, misleading diagnostic message) | open, upstream has a detailed root-cause writeup | (a) copy-then-read with a retry when the DB is locked, with a message pointing at "close Chrome" or "run as admin for VSS"; (b) `rookie` replaces `browser_cookie3` (supports newer Chrome cookie formats); (c) per-OS diagnostics (Windows: DPAPI/admin/shadowcopy; macOS: Keychain; Linux: keyring daemon); (d) the manual cookie-paste fallback is always available. The Windows leg of the P0 spike is mandatory specifically because of this issue. |
| #21 | Support official API v2 as an auth option (ban risk of cookie auth) | open, low priority | Phase 4's `--backend api-v2` (per PR #31): OAuth2 user-context, dual-backend routing per command (cookie backend for feed/bookmarks, official API for post/search/user where it's available), a `--backend` flag with a configurable default. SKILL.md documents the ban-risk trade-off explicitly. |
| #13 | Video upload (MP4, INIT/APPEND/FINALIZE + STATUS polling) | open | `upload.rs` implements `media_category=tweet_video`, chunked APPEND, and async STATUS polling until `succeeded`, plus `--video/--file/--alt-text` flags; cookie-backend video ships in P2 (following the PR #41 pattern), official-API video in Phase 4 (per PR #31). Processing failures map to `media_upload_error` with a clear suggestion. |
| #9 | Expose the tool as a library, not just a CLI | open | The Rust equivalent: every workspace crate is a real library (`twr-client`, `twr-model`, `twr-auth` export a stable public API with docs.rs coverage); a PyO3/FFI binding is explicitly out of scope. CLI commands are thin wrappers over a stable library API (e.g. `TwClient::fetch_home_timeline(…)`, mirroring the Python `TwitterClient`), with `examples/` and a `LIB.md`. |

## 11. Non-goals — what `twr` deliberately does NOT do

`twr`'s end-user value is real-world automation like a daily news-digest bot: fetch → filter → summarize → compose → post, running unattended on a schedule. **That workflow is achievable on top of `twr`, but none of its non-transport steps belong inside `twr` itself.** This is a scope boundary, not an oversight — write it down so nobody (including a future implementation pass) quietly grows `twr` into a bot framework and breaks the "clean primitive" property that makes it composable with *any* orchestrator, not just one opinionated pipeline.

```
   X/Twitter
       │
   twr (this repo) ── read: feed/search/bookmarks/user/list → structured JSON/YAML/TOON
       │             ── write: post/reply/like/... → --dry-run preview, --apply to execute
       │             ── twr knows nothing about "what's newsworthy" or "how to write a caption"
       ▼
   an external orchestrator (a separate project/script/skill, NOT a twr crate)
       │  - decides which tweets matter (dedup, ranking beyond §7's engagement-score filter)
       │  - calls an LLM to summarize/compose
       │  - decides *when* to post (cron/scheduler)
       │  - holds the human-approval step, if any, before --apply
       ▼
   twr post "..." --apply --idempotency-key <uuid>
```

Concretely, out of scope for `twr` itself:
- **Summarization / composition** — no LLM calls inside `twr`. It hands back structured tweet data; turning that into "here are today's 5 AI stories" prose is the orchestrator's job.
- **Newsworthiness ranking beyond `--filter`'s engagement score** (§7 `twr-filter`) — dedup-across-sources, topic clustering, editorial judgment are orchestrator concerns.
- **Scheduling / cron** — `twr` has no daemon mode. The P5 `future`/schedule idea (§9) is a thin one-shot delay, not a cron replacement; a real daily job belongs in the user's own scheduler (cron, systemd timer, GitHub Actions, a `/loop`-style agent skill) invoking `twr` as a subprocess or MCP tool.
- **Approval workflows beyond the `--apply`/`--dry-run`/prompt mechanics already in §5.3** — if an orchestrator wants Slack-approval-before-posting, that's orchestrator plumbing, not a `twr` flag.

Why this boundary matters: it's the same reason `--policy`/`--dry-run`/idempotency exist — `twr` is meant to be the safe, boring, well-tested layer that *any* automation (a daily-digest bot today, a completely different workflow tomorrow) can build on without `twr` ever needing to change. The moment `twr` starts making editorial decisions, it stops being a reusable primitive and becomes a single-purpose bot that happens to be written in Rust.

## 12. Risks

1. Cross-platform cookie decryption — validated in the P0 spike, with the cookie-paste fallback (Method C) as a permanent safety net.
2. Transaction-ID / query-ID rot — `doctor --refresh`, an unambiguous exit code 6, `TWR_QID_*` manual pins.
3. `wreq` gets fingerprinted and blocked → the `curl-impersonate` subprocess transport is a drop-in fallback via the `HttpTransport` trait.
4. Ban/rate-limit exposure — small defaults, explicit proxy guidance; never ship bulk-friendly defaults.
5. Binary naming: `twr` is primary, `twitter` is an opt-in alias.

## 13. Phase P6 — feature-completeness batch (post-v0.1.0)

v0.1.0 covers the "safe daily-digest bot" surface (§11). This phase closes the
gap to "a real Twitter client, minus the parts that are structurally
out-of-scope" (monetization/Ads/Jobs/Grok/Spaces-live — see §13.5). Query IDs
below are **starting points mined from public reference implementations**
(`xeepy`/XActions, `Rettiwt-API`, `agentic-x`, `TwitterInternalAPIDocument`),
not verified against this repo's own `doctor --refresh` — they rot every 2–4
weeks (§12 risk 2), so every bead here must re-resolve via the 4-layer
resolver before landing, never hardcode-and-ship blind.

Ordered by ban-risk (lowest first) and dependency, not by user-perceived
value — read-only ops first, mutations behind existing `--policy`/`--apply`
gates, the two DM ops last because they are the highest-scrutiny surface on
the platform (unsolicited automated DMs are explicitly named as a suspension
trigger — see the ban-risk research folded into the FAQ/README).

### 13.1 Read-only additions (lowest risk — extend existing GET-timeline pattern)

| Command | Operation(s) | Notes |
|---|---|---|
| `twr mentions` | `NotificationsTimeline` (or `UserTweetsAndReplies` on self scoped to mentions, whichever `doctor --refresh` resolves cleanly) | New timeline family; reuse `twr-graphql::resolve` + `twr-model` timeline parser, same pagination shape as `feed`. |
| `twr notifications` | `NotificationsTimeline` | Separate from mentions (likes/RTs/follows notify too); needs its own model struct — notification events aren't `Tweet`s. |
| `twr user-replies` | `UserTweetsAndReplies` (already gated by tx-id, same wall as `feed --latest`) or `UserRepliesTimeline` fallback (ungated per `agentic-x` research — prefer this one, cheaper) | Distinct from `user-posts`; some forks split "posts" vs "posts+replies" as two ops, verify both exist for our fallback chain. |
| `twr user-media` | `UserMedia` | Photo/video-only tab; same User timeline parser, filtered server-side. |
| `twr lists` | `List`/`Lists` (owned+followed) | Prerequisite for `list-create`/`list-add` below — needs to resolve list IDs the user owns before mutating them. |
| `twr list-members <list_id>` | `ListMembers` | Read-only, pairs with `list-add-member`/`list-remove-member`. |
| Bookmark folders (`twr bookmarks --folder <id>`) | `BookmarkFoldersSlice` / `BookmarkFolderTimeline` | Query IDs already in `consts.rs` (`FALLBACK_QUERY_IDS`) — command surface was never wired up. Cheapest item in this whole phase; do first. |

### 13.2 Engagement-tier mutations (same risk class as existing like/retweet/follow — gate behind `--policy engagement`)

| Command | Operation(s) | Notes |
|---|---|---|
| `twr mute <user_id>` / `twr unmute` | `MuteUser` / `UnmuteUser` | Same shape as existing `follow`/`unfollow` write commands; add to `twr-graphql/consts.rs` FALLBACK_QUERY_IDS. |
| `twr block <user_id>` / `twr unblock` | `BlockUser` / `UnblockUser` | Higher visible-effect mutation than mute (target sees blocked state) — keep under `--policy engagement`, not `read_only`. |
| `twr pin <tweet_id>` / `twr unpin` | (PinTweet mutation — resolve op name via `doctor --refresh`, not in current research corpus) | Low-volume, low-risk single-tweet mutation, same idempotency story as `like`. |

### 13.3 List management (write) — needs list-ownership read (13.1) landed first

| Command | Operation(s) | Notes |
|---|---|---|
| `twr list-create` | `CreateList` | Standard create-with-name/description/private mutation. |
| `twr list-edit` | `UpdateList` | Rename/redescribe/toggle private. |
| `twr list-delete` | `DeleteList` | Destructive — require `--apply`, always preview first like `delete` tweet does. |
| `twr list-add-member` / `twr list-remove-member` | `AddListMember` / `RemoveListMember` | Bulk-add is a spam vector (mass-list-adds get reported); no batch flag, one user per invocation, let the orchestrator loop with its own delay. |
| `twr list-follow` / `twr list-unfollow` | `FollowList` / `UnfollowList` | Subscribing to someone else's list — engagement-tier risk. |
| `twr list-pin` / `twr list-unpin` | `PinList` / `UnpinList` | Cosmetic, lowest risk in this group. |

### 13.4 Content-shape extensions (write, existing `post`/`quote` risk class)

| Command | Operation(s) | Notes |
|---|---|---|
| `twr post --long` (auto-route when `text` exceeds the standard weighted-length threshold) | `CreateNoteTweet` | Upstream Python issue #54 (README lists this as out of scope for v0.1.0 — supersede that note once this lands). Mirror upstream PR #64/#65's fixes exactly: (a) route both `create_tweet` **and** `quote_tweet` through `CreateNoteTweet` when over-length, not just plain posts; (b) must send `disallowed_reply_options: null` explicitly — omitting it silently returns an empty `tweet_results` (documented failure mode in PR #64); (c) parse response tolerant of all three envelope shapes seen in the wild (`create_tweet` / `notetweet_create` / `create_note_tweet`); (d) fail closed (explicit error, not silent success) if the mutation returns no confirmation. |
| `twr edit <tweet_id>` | `EditTweet` (`responsive_web_edit_tweet_api_enabled` — already `true` in our `DEFAULT_FEATURES`) | X gates this to Premium/Blue accounts server-side with a short edit window and edit-count limit; `twr` must surface X's own error cleanly (not retry) when the account isn't eligible — don't invent client-side eligibility checks that will drift from X's actual policy. |
| `twr post --poll` | Poll is a `card_uri`/`poll` field on the existing `CreateTweet`/`CreateNoteTweet` mutation, not a separate op | Confirm exact variable shape against `doctor --refresh` capture before shipping — the official-API-v2 OpenAPI schema (`poll.options[2..4]`, `poll.duration_minutes[5..10080]`) is a decent cross-check for the field semantics even though the wire format differs from cookie-GraphQL. |

### 13.5 Direct Messages — last, highest scrutiny, opt-in only

| Command | Operation(s) | Notes |
|---|---|---|
| `twr dm-list` | `DmInbox` / `DMConversation` list | Read-only; still ship it after everything else in this phase because DM inbox structure (conversations, not tweets) needs its own model type and is the least code-reused item in the batch. |
| `twr dm-send <user_id> <text>` | `useSendMessageMutation` | **Must** default to `--policy write`-only (never `engagement`), require `--apply`, and get its own line in SKILL.md's ban-risk section: unsolicited automated DMs are one of the clearest platform suspension triggers (see FAQ/README ban-risk research). Consider a hard per-day cap independent of the existing mutation budget. |

### 13.6 Explicitly still out of scope (unchanged from §11, restated for this phase)

Subscriptions/monetization, Ads, Jobs, Grok (assistant surface), Spaces
create/host/live-audio, and Communities create/moderate are **not** part of
P6. Rationale: each is either (a) a monetization/business surface with no
"daily digest bot" use case `twr` targets, (b) requires account tiers/UI
flows that don't map to a stable scriptable contract, or (c) — for
Spaces/Communities specifically — is real-time/stateful in a way that doesn't
fit `twr`'s one-shot-command model at all. If a real user need for one of
these surfaces shows up, it gets its own proposal + risk writeup, not a quiet
addition to this table.

### 13.7 Sequencing

1. Bookmark folders command wiring (13.1, query IDs already shipped — zero new resolver work).
2. Read-only additions (13.1) — establishes `NotificationsTimeline`/list-read patterns the write commands in 13.3 depend on.
3. Mute/block/pin (13.2) — smallest new-mutation surface, reuses the `follow`/`like` code path almost verbatim.
4. List management (13.3) — depends on 13.1's list-read commands for ID resolution in tests/docs.
5. Long-form + edit + poll (13.4) — highest implementation complexity (three response-envelope shapes for note-tweet alone), do after the team has re-proven the query-ID resolver process on simpler ops in 1–4.
6. DM (13.5) — last, gated on its own SKILL.md ban-risk writeup being reviewed before `--apply` ships for `dm-send`.
