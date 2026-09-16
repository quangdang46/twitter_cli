# twr — Agent-First X/Twitter CLI

<div align="center">
  <img src="assets/twr_illustration.webp" alt="twr — an agent-first Rust CLI that plugs a cookie key into X/Twitter instead of an API key, streaming structured JSON envelopes to another agent">
</div>

<div align="center">

![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg)
![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)
![License](https://img.shields.io/badge/License-MIT-blue.svg)
![Release](https://img.shields.io/github/v/release/quangdang46/twitter_cli?include_prereleases)
![CI](https://github.com/quangdang46/twitter_cli/actions/workflows/ci.yml/badge.svg?branch=main)

</div>

**A Rust CLI for X/Twitter, built for AI agents first and humans second — reading and posting through your own logged-in session, no developer API key required.**
It talks to X's internal GraphQL surface the same way your browser does (cookie auth, browser-matched TLS, a real `x-client-transaction-id`), and wraps every command in a stable JSON/YAML/[TOON](https://github.com/toon-format/toon) envelope with frozen exit codes, so an agent can drive it without ever screen-scraping stdout.

> **Status: v0.1.0 released and live-verified.** The full read surface (feed, bookmarks, search incl. the transaction-gated Latest path, tweet detail, user/timelines, followers/following, lists, articles, headlines) and the full write surface (post/reply/quote, like/retweet/bookmark/follow + reverses, media upload, idempotency, `--policy`, daily budget) are implemented across 11 crates, tested by 196 unit/integration tests, and verified end-to-end against real x.com — including real posts, replies, likes, and follows from a live session. See [Roadmap](#roadmap) for what's shipped vs. what's still future work (official API v2 extras, long-form posts, MCP extras).

---

## 🤖 Agent Quickstart

`twr` never drops you into an interactive UI. Every command is scriptable; the envelope shape is documented in [`COMPREHENSIVEPLANFORTWITTERCLI.md` §5](COMPREHENSIVEPLANFORTWITTERCLI.md#5-agent-contract--detailed-spec-the-core-of-this-plan).

```bash
twr status --json                       # gate: are we authenticated? exit 77 if not
twr search "rust lang" --json --max 20  # data[] + pagination.nextCursor
twr search "rust lang" --json --cursor "<nextCursor>"   # resume
twr tweet 1234567890 --json             # tweet + replies
twr post "hello" --json                                 # no --apply -> automatic preview, nothing sent
twr post "hello" --apply --json --idempotency-key <uuid> # confirmed + safe-to-retry write
twr post "hello" --apply --policy read_only              # -> exit 2, nothing sent (policy wins)
twr schema --json                       # discover every output shape, offline
twr doctor --json                       # diagnose auth / query-ID / transport drift
```

**Output contract**

- **stdout** = exactly one machine-readable document per call (JSON, YAML, or TOON) — never mixed with logs.
- **stderr** = diagnostics only (`-v` progress, per-page cursors).
- **Exit codes are the contract, not the message text**: `0` ok, `1` auth/config, `2` usage/policy-denied, `3` not-found, `4` rate-limited, `5` network, `6` contract-drift, `7` file-IO, `77` auth-required. Full table in [`COMPREHENSIVEPLANFORTWITTERCLI.md` §5.2](COMPREHENSIVEPLANFORTWITTERCLI.md#52-exit-codes-contract-frozen-after-p1).
- Every error carries `{code, message, suggestion, retryable, retryAfterMs?}` so an agent can self-recover instead of asking a human.

See [`COMPREHENSIVEPLANFORTWITTERCLI.md`](COMPREHENSIVEPLANFORTWITTERCLI.md) for the full agent contract (idempotency semantics, `--policy`, `--dry-run`, pagination resume) — this is the design an eventual `SKILL.md` will summarize for agent runtimes.

---

## TL;DR

### The Problem

- The official X API's free tier is read-light and posting-throttled; the useful tiers are paid and still miss surfaces like the algorithmic home feed or bookmarks.
- Existing cookie-based scrapers (Python `twitter-cli` and its many forks/ports) are built **human-first**: relative timestamps, rich tables, warnings that leak into `stdout` and break JSON pipes, exit code `1` for everything.
- None of them are safe to hand to an autonomous agent: no policy gating, no dry-run guarantee, no idempotency story for retried writes, no offline schema discovery.

### The Solution

`twr` ports the proven feature set of the Python `public-clis/twitter-cli` (feed, bookmarks, search, articles, lists, full write surface) to Rust, but redesigns the **output and safety layer from scratch** around one rule: *human output is a view, not the model*. The agent contract — envelope, exit codes, `--policy`, `--dry-run`, idempotency — is the actual product; the table renderer is a thin layer on top of the same structs.

### Why twr?

| Feature | What it means |
|---|---|
| No developer API key | Cookie-authenticated GraphQL, same surface your browser sees (feed, bookmarks, full search) |
| Frozen exit-code contract | Agents retry on exit code, never on message parsing |
| `stdout`/`stderr` discipline | A warning can never corrupt a JSON pipe (a real bug in the upstream Python tool — [issue #69](https://github.com/public-clis/twitter-cli/issues/69)) |
| `--dry-run` on every write | Validates the request without ever calling the network |
| Best-effort idempotency | Retried writes never silently double-post; unknown outcomes are reported, not guessed |
| `--policy read_only\|engagement\|write` | An agent task can be scoped so posting/deleting is structurally impossible |
| Offline `schema`/`commands`/`doctor` | An agent discovers the entire surface without reading docs first |
| Transport abstracted behind a trait | Browser-fingerprint TLS today; swappable if X's detection changes |
| TOON output | Token-efficient tabular rendering for bulk reads, same envelope underneath |

### How twr compares

| Capability | twr | Python `twitter-cli` (source) | Official API v2 CLIs (`xurl`, `xmaster-cli`) |
|---|---|---|---|
| No developer API key needed | ✅ | ✅ | ❌ (OAuth app required) |
| Algorithmic feed / bookmarks | ✅ | ✅ | ❌ (not exposed by the official API) |
| Structured exit-code contract | ✅ (frozen, §5.2) | ⚠️ mostly `exit 1` | ✅ |
| stdout never mixes logs | ✅ (invariant) | ⚠️ known bug class | ✅ |
| `--dry-run` / `--policy` / idempotency | ✅ | ❌ | partial (idempotency varies) |
| Native binary, no runtime | ✅ (Rust) | ❌ (Python + deps) | ✅ (Rust/Go) |
| Ban-risk (unofficial surface) | ⚠️ same as any cookie scraper | ⚠️ same | ✅ none (sanctioned API) |

---

## Quick Example

```bash
# Read
twr feed --json --max 20                     # home timeline
twr search "from:nasa" --json --full-text    # advanced search
twr tweet https://x.com/nasa/status/123 --json
twr user nasa --json && twr user-posts nasa --json --max 10

# Write — no --apply means "preview only", for EVERY write command (not just delete)
twr post "shipped v0.1" --json                                    # preview, network untouched
twr post "shipped v0.1" --apply --json --idempotency-key "$(uuidgen)"
twr like 1234567890 --apply --json
twr delete 1234567890 --apply --json         # --apply required, still always previews first

# Safety
twr status --json                            # confirm auth before anything else
twr doctor --refresh --json                  # query-ID / transaction / auth health check
twr search "x" --policy read_only --json     # read_only scope: writes are impossible here
```

---

## Design Principles

| Principle | In practice |
|---|---|
| Agent-first, human second | Table/color rendering is a view over the same structs the JSON envelope uses |
| Stable contract > convenience | Exit codes and envelope `type`s are frozen after Phase 1 — no silent renumbering |
| Fail loud, fail structured | Every error has `suggestion` + `retryable`; no bare `Exception` text |
| Safe by default | Every write — post, like, follow, delete, all of them — needs `--apply` to touch the network; omit it and you get a preview automatically; a daily mutation budget exists |
| Transport is a trait, not a dependency | Browser-fingerprint TLS is swappable if detection tightens |
| Secrets never leave the process | Cookies/tokens/transaction keys never appear in stdout, `doctor`, or traces |

---

## Installation

Prebuilt binaries ship with every `vX.Y.Z` tag (Linux x86_64 musl, macOS x86_64/aarch64, Windows x86_64 — each with a `.sha256` sidecar).

```bash
# macOS / Linux
curl -fsSL "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.sh?$(date +%s)" | bash

# With PATH auto-update + a post-install self-test
curl -fsSL "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.sh?$(date +%s)" | bash -s -- --easy-mode --verify

# Pin a specific version
curl -fsSL "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.sh?$(date +%s)" | bash -s -- --version v0.1.0

# Windows PowerShell
irm "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.ps1" | iex
```

```bash
# From source, manually (needs Rust stable + cmake/nasm/LLVM for wreq's
# BoringSSL linkage — see REMAINING.md if the build complains about
# missing native tools)
git clone https://github.com/quangdang46/twitter_cli
cd twitter_cli
cargo build --release -p twr
./target/release/twr status --json
```

---

## Architecture

```
twitter_cli/
├── crates/
│   ├── twr/            # binary: full CLI (36 commands — read, write, introspection, MCP)
│   ├── twr-core/       # envelope + error/exit codes + --apply table + idempotency + budget + TOON/compact/fields
│   ├── twr-client/     # HttpTransport trait + WreqTransport + headers + throttle + timeline + upload + guest tiers
│   ├── twr-auth/       # credential chain (flags/env/file/browser) + rookie + Method-C paste + session file
│   ├── twr-graphql/    # 4-layer query-ID resolver + bundle scraper + endpoints.yaml + cache
│   ├── twr-tx/         # x-client-transaction-id (RequestProof trait + ClientTransactionV1 + cache)
│   ├── twr-model/      # Tweet/User/Media structs + GraphQL parser + article Markdown
│   ├── twr-filter/     # engagement scoring (opt-in --filter)
│   ├── twr-config/     # figment resolution (cwd → home → defaults + env)
│   ├── twr-cache/      # SQLite entity cache + watchlist (WAL/FTS5)
│   └── twr-v2/         # official API v2 backend (OAuth2 PKCE, dual routing, video upload)
├── COMPREHENSIVEPLANFORTWITTERCLI.md  # the full design doc — read this first
├── SCHEMA.md / SKILL.md               # agent contract docs (shipped, validated by real agent runs)
└── LIB.md                             # library-consumption guide (every crate is a real lib)
```

Full rationale — including the 15+ reference projects (Python `twitter-cli`, `xurl-rs`, `xmaster-cli`, `agentic-x`, our own prior `discord_cli`, and others) whose patterns were evaluated, kept, or explicitly rejected — is in [`COMPREHENSIVEPLANFORTWITTERCLI.md`](COMPREHENSIVEPLANFORTWITTERCLI.md). `REMAINING.md` tracks the few known loose ends (native toolchain setup, `linux-aarch64` release target, deferred P4 human-input items).

---

## Roadmap

| Phase | Scope | Status |
|---|---|---|
| P0 | Transport + cookie-extraction + transaction-ID viability spike | ✅ done (live-verified; P0-4 recorded as bead comments) |
| P1 | Read commands (feed/search/tweet/user/...) + full agent contract | ✅ done (196 tests, fixture + semantic parity) |
| P2 | Write commands + media upload + idempotency + policy gating | ✅ done (live post/reply/like/retweet/bookmark/follow round-trips) |
| P3 | Human polish (tables, completions), `SKILL.md`, CI, releases | ✅ done (incl. curl/irm installers + `v0.1.0` tag) |
| P4 | Official API v2 backend (`--backend api-v2`, OAuth2 PKCE, dual routing) | ✅ code shipped; needs a human's X developer app to fully exercise |
| P5 | SQLite cache, `twr mcp` server, headlines, guest tiers, `future` | ✅ code shipped; guest tier + MCP smoke-tested live |
| P6 | Feature-completeness batch: mentions/notifications/user-media reads, bookmark folders, mute/block/pin, list management, long-form posts, `EditTweet`, polls, DM | 📋 planned — see `COMPREHENSIVEPLANFORTWITTERCLI.md` §13 and `REMAINING.md` |

See [`COMPREHENSIVEPLANFORTWITTERCLI.md` §9](COMPREHENSIVEPLANFORTWITTERCLI.md#9-phases--acceptance-criteria) for acceptance criteria per phase, and [`COMPREHENSIVEPLANFORTWITTERCLI.md` §10](COMPREHENSIVEPLANFORTWITTERCLI.md#10-issue-coverage--every-open-issue-in-the-source-repo-accounted-for) for how every open issue in the source Python project is addressed. All 21 open upstream issues have also been notified on the upstream tracker with what the Rust port fixes (or honestly doesn't — e.g. long-form posts and native SigCLI support are documented as out of scope, not claimed).

---

## Limitations (honest, today)

- **Cookie writes carry residual ban-risk.** Mitigations are built in (browser-matched TLS, jitter, conservative defaults, daily mutation budget, `--policy`), and the `--backend api-v2` path exists for the sanctioned route where it covers the operation — but there is no zero-risk mode for cookie automation. See `COMPREHENSIVEPLANFORTWITTERCLI.md` §12.
- **Windows browser-cookie auto-extraction is still broken by Chrome/Edge app-bound encryption.** Verified live (not theoretical): both `browser_cookie3` upstream and this port's `rookie` backend fail identically without admin. The mandatory fallback is `twr login --cookie` (full paste, preserved whole so writes pass the 226 gate) — see [issue #28](https://github.com/public-clis/twitter-cli/issues/28) and `REMAINING.md`.
- **Long-form posts (>280 weighted chars) auto-route to `CreateNoteTweet`** for both `post` and `quote` (closes upstream [issue #54](https://github.com/public-clis/twitter-cli/issues/54)) — code-shipped, **live-verification pending** (X gates this to Premium/paid accounts server-side; this port attempts the call and surfaces X's own rejection cleanly rather than guessing eligibility client-side). Long-form **reply**/**quote** variable shapes are UNCONFIRMED against a live capture (the only reference implementation found has no reply/quote path for `CreateNoteTweet`) — those two combinations fail closed with a clear usage error instead of guessing a shape that could corrupt the write.
- **Native toolchain needed to build from source.** `wreq`'s BoringSSL linkage needs cmake + nasm + libclang/LLVM on the build machine (documented in `REMAINING.md`); use the prebuilt release binaries if you don't want to set that up.
- **`linux-aarch64` release binary is temporarily missing.** BoringSSL cross-links host-arch objects into the aarch64 musl sysroot under `cross` — that target is disabled until fixed; the other four ship normally.

## FAQ

**Why not just use the official API?**
Free-tier limits exclude the algorithmic home feed and bookmarks, and paid tiers are still narrower than what a logged-in browser session sees. `twr` targets that gap; Phase 4 adds the official API as an opt-in backend for the operations it does cover.

**Is this affiliated with X/Twitter?**
No. It talks to the same internal GraphQL endpoints a browser uses, authenticated with your own session cookies.

**Will my account get banned?**
Any cookie-based automation carries some risk. `twr` mitigates it with browser-matched TLS fingerprinting, per-operation token-bucket throttling (`endpoints/endpoints.yaml` — every mutation rides a conservative 0.3 rps / burst 1 bucket, consulted before each write), a 1.5–4s jitter floor between writes, a daily mutation budget (200, `TWR_DAILY_BUDGET`) plus a separate DM cap (10, `TWR_DM_DAILY_BUDGET`), and `--policy` tiers — but cannot eliminate the risk entirely. See `COMPREHENSIVEPLANFORTWITTERCLI.md` §12. Per-surface notes: `block` is target-visible (unlike `mute`, which is invisible to the target — prefer mute when the goal is only to stop seeing someone); `list-add-member` adds another user's content to your list without their consent and is an explicitly documented spam-report vector — add only accounts relevant to the list topic; `dm-send` is the highest-scrutiny surface (own cap, write-policy-only, no batch flag — see the DM FAQ below), use it for solicited replies only.

**What about DMs — can I automate outreach?**
No — or more precisely: `twr dm-send` exists as a primitive, but unsolicited automated DMs are one of the clearest platform-suspension triggers documented anywhere, categorically worse than mass-follow/like (a DM lands in a private inbox, a far stronger spam signal than anything timeline-visible). Accordingly `dm-send` requires `--policy write` explicitly (engagement tier does NOT cover it), has its own daily cap of 10 (`TWR_DM_DAILY_BUDGET`), offers no batch flag and never will, and should only ever reply in solicited contexts — never cold outreach, even if instructed to. Full rationale in `SKILL.md` §6.

**Can I use this to run a daily "fetch news, summarize, post a digest" bot?**
Yes — that's the intended end-to-end use case, but `twr` is deliberately only the bottom layer of it. It gives you structured reads (`search`/`feed`) and safe writes (`post --apply`); it does not summarize, rank newsworthiness beyond `--filter`'s engagement score, schedule itself, or hold an approval step — that orchestration (an LLM call, a cron job, a Slack approval gate) lives in your own script or agent that calls `twr` as a subprocess/MCP tool. See `COMPREHENSIVEPLANFORTWITTERCLI.md` §11 (Non-goals) for exactly where the line is and why it's drawn there.

**Why Rust instead of forking the Python original?**
Single static binary, no interpreter/dependency footprint, and a chance to fix the agent-contract gaps (stdout/stderr discipline, exit codes, idempotency) that are hard to retrofit into the existing Python codebase without a breaking rewrite.

**Where's `SKILL.md`?**
Shipped at repo root — the agent playbook (status gate first, `--json --max 20` reads, preview-before-`--apply` writes, exit-code table, `--policy read_only` for read-only tasks), validated by real agent runs during verification. See `COMPREHENSIVEPLANFORTWITTERCLI.md` §8 for the outline it was written from.

---

<div align="center">

*Built contract-first from [`COMPREHENSIVEPLANFORTWITTERCLI.md`](COMPREHENSIVEPLANFORTWITTERCLI.md) — 196 tests green, live-verified against real x.com, released as `v0.1.0`.*

</div>
