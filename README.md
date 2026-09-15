# twr — Agent-First X/Twitter CLI

<div align="center">
  <img src="assets/twr_illustration.webp" alt="twr — an agent-first Rust CLI that plugs a cookie key into X/Twitter instead of an API key, streaming structured JSON envelopes to another agent">
</div>

<div align="center">

![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-blue.svg)
![Rust](https://img.shields.io/badge/Rust-stable-orange.svg)
![License](https://img.shields.io/badge/License-MIT-blue.svg)
![Status](https://img.shields.io/badge/status-pre--implementation%20(P0)-yellow.svg)

</div>

**A Rust CLI for X/Twitter, built for AI agents first and humans second — reading and posting through your own logged-in session, no developer API key required.**
It talks to X's internal GraphQL surface the same way your browser does (cookie auth, browser-matched TLS, a real `x-client-transaction-id`), and wraps every command in a stable JSON/YAML/[TOON](https://github.com/toon-format/toon) envelope with frozen exit codes, so an agent can drive it without ever screen-scraping stdout.

> **Status: pre-implementation.** This repo currently ships the design (`COMPREHENSIVEPLANFORTWITTERCLI.md`) plus a minimal Cargo workspace that compiles (`twr status` / `twr schema` stubs). No network code exists yet — see [Roadmap](#roadmap) for what's real today vs. planned.

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

> No tagged release exists yet — Phase 0/1 is still in progress, so the installer below will fall back to building from source (needs `cargo`) until the first `vX.Y.Z` tag ships prebuilt binaries via CI.

```bash
# macOS / Linux
curl -fsSL "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.sh?$(date +%s)" | bash

# With PATH auto-update + a post-install self-test
curl -fsSL "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.sh?$(date +%s)" | bash -s -- --easy-mode --verify

# Windows PowerShell
irm "https://raw.githubusercontent.com/quangdang46/twitter_cli/main/install.ps1" | iex
```

```bash
# From source, manually
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
│   ├── twr/            # binary: CLI entry point (clap)
│   ├── twr-core/        # envelope + error/exit-code contract (implemented, tested)
│   ├── twr-client/      # HttpTransport trait + GraphQL client (stub — P0/P1)
│   ├── twr-auth/        # cookie resolution + browser extraction        (planned, P1)
│   ├── twr-graphql/     # query-ID resolver + bundle scraping           (planned, P1)
│   ├── twr-tx/          # x-client-transaction-id derivation            (planned, P0)
│   ├── twr-model/       # Tweet/User/Media structs + parser             (planned, P1)
│   ├── twr-filter/      # engagement scoring                            (planned, P3)
│   ├── twr-output/      # table/YAML/TOON rendering                     (planned, P3)
│   └── twr-config/      # figment-based config resolution               (planned, P1)
├── COMPREHENSIVEPLANFORTWITTERCLI.md              # the full design doc — read this first
└── SCHEMA.md / SKILL.md # written once the core contract stabilizes (P3)
```

Full rationale — including the 15+ reference projects (Python `twitter-cli`, `xurl-rs`, `xmaster-cli`, `agentic-x`, our own prior `discord_cli`, and others) whose patterns were evaluated, kept, or explicitly rejected — is in [`COMPREHENSIVEPLANFORTWITTERCLI.md`](COMPREHENSIVEPLANFORTWITTERCLI.md).

---

## Roadmap

| Phase | Scope | Status |
|---|---|---|
| P0 | Transport + cookie-extraction + transaction-ID viability spike | ⏳ not started |
| P1 | Read commands (feed/search/tweet/user/...) + full agent contract | ⏳ not started |
| P2 | Write commands + media upload + idempotency + policy gating | ⏳ not started |
| P3 | Human polish (tables, completions), `SKILL.md`, CI, releases | ⏳ not started |
| P4 | Official API v2 backend (feature-gated), extras (MCP, SQLite cache) | ⏳ not started |

See [`COMPREHENSIVEPLANFORTWITTERCLI.md` §9](COMPREHENSIVEPLANFORTWITTERCLI.md#9-phases--acceptance-criteria) for acceptance criteria per phase, and [`COMPREHENSIVEPLANFORTWITTERCLI.md` §10](COMPREHENSIVEPLANFORTWITTERCLI.md#10-issue-coverage--every-open-issue-in-the-source-repo-accounted-for) for how every open issue in the source Python project is addressed.

---

## Limitations (honest, today)

- **Nothing talks to X yet.** This is a design + scaffold repository; `twr status`/`twr schema` are stubs proving the envelope contract, not real auth checks.
- **Unofficial surface.** Cookie-based GraphQL access carries the same account-risk profile as any scraper — see `COMPREHENSIVEPLANFORTWITTERCLI.md` §12 for mitigations, but there is no zero-risk mode until the Phase 4 official-API backend lands.
- **Windows cookie extraction is the highest-risk unknown.** The upstream Python tool has an open, well-documented failure mode here ([issue #28](https://github.com/public-clis/twitter-cli/issues/28)); the P0 spike exists specifically to validate a fix before committing to the full port.

## FAQ

**Why not just use the official API?**
Free-tier limits exclude the algorithmic home feed and bookmarks, and paid tiers are still narrower than what a logged-in browser session sees. `twr` targets that gap; Phase 4 adds the official API as an opt-in backend for the operations it does cover.

**Is this affiliated with X/Twitter?**
No. It talks to the same internal GraphQL endpoints a browser uses, authenticated with your own session cookies.

**Will my account get banned?**
Any cookie-based automation carries some risk. `twr` mitigates it with browser-matched TLS fingerprinting, request jitter, and conservative default rate limits — see `COMPREHENSIVEPLANFORTWITTERCLI.md` §12 — but cannot eliminate the risk entirely.

**Can I use this to run a daily "fetch news, summarize, post a digest" bot?**
Yes — that's the intended end-to-end use case, but `twr` is deliberately only the bottom layer of it. It gives you structured reads (`search`/`feed`) and safe writes (`post --apply`); it does not summarize, rank newsworthiness beyond `--filter`'s engagement score, schedule itself, or hold an approval step — that orchestration (an LLM call, a cron job, a Slack approval gate) lives in your own script or agent that calls `twr` as a subprocess/MCP tool. See `COMPREHENSIVEPLANFORTWITTERCLI.md` §11 (Non-goals) for exactly where the line is and why it's drawn there.

**Why Rust instead of forking the Python original?**
Single static binary, no interpreter/dependency footprint, and a chance to fix the agent-contract gaps (stdout/stderr discipline, exit codes, idempotency) that are hard to retrofit into the existing Python codebase without a breaking rewrite.

**Where's `SKILL.md`?**
Written in Phase 3, once the envelope/exit-code contract is frozen and validated against a real agent — see `COMPREHENSIVEPLANFORTWITTERCLI.md` §8.

---

<div align="center">

*Designed contract-first: the plan in [`COMPREHENSIVEPLANFORTWITTERCLI.md`](COMPREHENSIVEPLANFORTWITTERCLI.md) is the source of truth until the code catches up to it.*

</div>
