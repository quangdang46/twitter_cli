# REMAINING — things not yet finished/verified, tracked outside the beads graph

A running log of loose ends discovered while implementing
`COMPREHENSIVEPLANFORTWITTERCLI.md` that don't cleanly map to "close this bead"
but need to be revisited. Check this file before assuming the workspace is in
a fully clean state.

## Toolchain / build environment (this dev machine)

- **`crates/twr-client` needs a working native toolchain for wreq's BoringSSL
  linkage** (via `boring-sys`): `cmake` (installed via `winget install
  Kitware.CMake`) + `nasm` (winget's `NASM.NASM` package silently failed to
  register — `winget list` shows nothing installed despite a "Successfully
  installed" message; worked around by downloading the official zip release
  directly to `C:\Users\ADMIN\tools\nasm\nasm-2.16.03\nasm.exe` and adding it
  to `PATH` for the shell session) + **`libclang`/LLVM** (needed by
  `bindgen` to generate BoringSSL FFI bindings — `winget install LLVM.LLVM`
  was kicked off but not confirmed working as of this note).
- **Practical consequence**: `cargo build --workspace` / `cargo test
  --workspace` do NOT currently work end-to-end on a fresh checkout of this
  repo without first fixing this chain. Until this is resolved (or documented
  as a first-run setup step / addressed via a CI Docker image with the full
  toolchain baked in), always scope `cargo build/test/clippy` to the specific
  crates you're touching, e.g.:
  `cargo build -p twr-model -p twr-tx -p twr-core -p twr-auth` (these four
  have zero native-toolchain dependencies and build cleanly everywhere).
- **Action item for whoever picks this up next**: either (a) get LLVM
  properly installed and confirm `cargo build -p twr-client` links end to
  end, or (b) reconsider the plan's transport choice — the `HttpTransport`
  trait (crates/twr-client/src/lib.rs) was specifically designed so a
  `CurlImpersonateTransport` (subprocess-based, no BoringSSL linking needed
  in-process) can be swapped in without touching call sites, per plan §11
  risk #3. If native-toolchain friction turns out to be a recurring
  contributor-onboarding problem, that fallback might deserve to become the
  *default*, not just an escape hatch.

## P0-1 (wreq transport spike) — code done, SUPERSEDED by live proof

The `#[ignore]`d `tls.peet.ws` fingerprint-inspector test in
`crates/twr-client/src/wreq_transport.rs` was never run standalone — and no
longer needs to be. The toolchain got fixed (cmake + direct-zip nasm +
background-installed LLVM, all confirmed working: full `cargo build/test
--workspace` green), and something strictly stronger than the neutral-probe
test has since happened repeatedly: **real, authenticated GraphQL calls
against actual x.com through this exact `WreqTransport`** (feed, search incl.
the transaction-gated Latest path, tweet detail, user lookups, follows,
posts, likes, retweets, bookmarks — plus a real post/reply/like/follow
round-trip from a live session). If the TLS fingerprint were not
browser-shaped, none of that would return 200. Keep the ignored test as a
cheap regression probe for contributor machines, but do not treat its
never-having-run as an open verification gap.

## P0-4 (end-to-end call) — DONE, exceeded original scope

History: this was deliberately deferred (a human explicitly supplied a real
x.com cookie string via Method C, which is the only correct way that step
could happen). What actually ran went **beyond** the bead's original scope
(UserByScreenName-only): a full live round trip — login → dry-run preview
→ real post with idempotency key → read-back of the same tweet with
matching text/author/URL/timestamp — plus subsequent live sessions covering
replies, likes, retweets, bookmarks, follows, feed, search (incl. gated
Latest), followers, headlines, and user timelines. Recorded as comments on
beads `twitter_cli-5o3.2.4`/`5o3.2.5`, upgrading the prior "GO conditional"
to unconditional. Nothing further to do here; keeping this note so nobody
re-opens the bead thinking the e2e was never run.

## Retained scope decisions worth double-checking later

- `crates/twr-model/src/parse.rs`'s `render_text_block` (Draft.js inline-link
  rendering) indexes by Rust `char`, matching the Python original's behavior
  of indexing by Python `str` code points — but Draft.js's own `offset`/
  `length` values are defined in **UTF-16 code units**. Both this port and
  the Python original inherit the same latent bug for non-BMP characters
  (emoji, some CJK) in article text; this was a pre-existing upstream
  limitation, not something introduced here, but worth fixing properly (UTF-16
  offset translation) if article rendering on emoji-heavy content ever comes
  up as a real bug report.

## P4 (official API v2 backend) — deferred, needs human decisions first

Beads `twitter_cli-5o3.6.1` (OAuth2), `6.2` (dual-backend routing), `6.3`
(video upload) were NOT implemented in the autonomous pass because each
needs an input only a human can provide:

1. **X developer account + app credentials.** OAuth2 user-context requires a
   real X developer app (client ID/secret, callback URL, approved access
   tier). No agent can or should create that.
2. **Backend-choice confirmation.** PR #31's dual-backend shape (which
   commands route where, `--backend` default) is a product decision with
   ban-risk trade-off implications (cookie vs. official API) that SKILL.md
   must then document honestly.
3. **Video fixtures.** Cookie-backend video (PR #41 pattern) also needs a
   throwaway account to verify INIT/APPEND/FINALIZE + STATUS polling.

When those exist, 6.1→6.2→6.3 implement in order behind
`--backend api-v2` (feature-gated, cookie backend stays default).

## P6 (feature-completeness batch) — in progress

`COMPREHENSIVEPLANFORTWITTERCLI.md` §13 has the full spec. Read-only
additions (mentions/notifications/user-media/user-replies/lists/list-members/
bookmark-folders), engagement mutations (mute/block/pin), and list management
(create/edit/delete/add-member/follow/pin) are code-shipped. Sequencing and
per-op query-ID starting points (mined from `xeepy`/`Rettiwt-API`/`agentic-x`)
are in §13.7 — most are still **unverified against this repo's own live
session**; several list mutations (`CreateList`/`ListSubscribe`/…) are
currently blocked live with X error 214 (DecodeException) despite vars/
features/queryId-in-body all matching a working Rettiwt-API reference
byte-for-byte — root cause unresolved, tracked as an open investigation
(DevTools capture of `x.com/i/lists/create` is the next planned step).

`CreateNoteTweet` (long-form `post`/`quote` auto-routing, supersedes the
"not implemented" note in README's Limitations) is code-shipped 1-1 against
a verified Rettiwt-API reference (`postNote`, blob `4f11105`, cross-checked
byte-for-byte by a second reviewer) — **live-verification still pending**
(needs a Premium-tier throwaway account). Long-form reply/quote variable
shapes are UNCONFIRMED (no reference implementation has a reply/quote path
for this mutation) and fail closed with a usage error rather than guess.

Explicitly excluded: monetization/Ads/Jobs/Grok/Spaces-live/
Communities-create (§13.6) — no "daily digest bot" use case, or
fundamentally stateful/real-time in a way `twr`'s one-shot command model
doesn't fit. `EditTweet`, poll creation, and DM (read + send) remain
not-yet-started.
