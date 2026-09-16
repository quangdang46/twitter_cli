# twr — Agent Skill

`twr` is an agent-first CLI for X/Twitter over cookie-authenticated GraphQL.
It is a safe read/write **primitive** — structured reads and gated writes —
never a summarizer, scheduler, or approval workflow (see GUARDRAIL bead
`twitter_cli-5o3.1`).

## 1. Gate first

```sh
twr status --json
```

- `authenticated: false` (exit 77) → run `twr login --guide --json` and
  follow one of the three methods (browser / env / cookie paste).
- Never echo secrets: cookies, `auth_token`, `ct0`, session file contents
  must not appear in output, logs, or `failingInput`.

## 2. Reading

Start narrow, page outward:

```sh
twr search "rust" --json --max 20
twr search "rust" --json --max 20 --cursor <nextCursor>
twr tweet <id-or-url> --json
twr show 3 --json        # Nth item of the last list (~/.twr/last.json)
```

- `--compact` strips heavy fields when context is tight.
- `--fields id,text,author.screen_name,metrics.likes` projects.
- `--filter` applies engagement scoring (opt-in, off by default).
- `--time absolute|both` affects only the human table; machine output
  always carries absolute `created_at`.

## 3. Writing

A write **without** `--apply` is already a safe preview (§5.3):

```sh
twr post "hello" --json            # ambiguous → exit 2, or TTY prompt
twr post "hello" --dry-run --json  # preview {dry_run:true,...}, no network
twr post "hello" --apply --idempotency-key <uuid> --json   # for real
```

- Show the user the preview, then re-run with `--apply --idempotency-key`.
- Writes need full browser cookies; env-only auth risks HTTP 226.
- `--policy read_only` blocks all writes (exit 2) — use it for read-only tasks.
- Daily mutation budget (default 200, `TWR_DAILY_BUDGET`) exits 2 when spent.

## 4. Errors → actions

| Exit | Meaning | Action |
|------|---------|--------|
| 0 | success (incl. dry-run) | parse `data` |
| 2 | usage/policy-denied | fix args or policy |
| 3 | not-found (target missing) | verify ID, do not retry |
| 4 | rate-limited | back off `retryAfterMs`, resume with `--cursor` |
| 5 | network | exponential-backoff retry |
| 6 | contract drift (stale query ID) | `twr doctor --refresh` once, retry once |
| 7 | file/attachment IO | fix path/permissions/size |
| 77 | auth required | `twr status` → login, do not blind-retry |

## 5. Safety

- `--policy read_only` for read-only tasks.
- No bulk operations; one write per invocation, budget-capped.
- Every write consults its per-operation token bucket (`endpoints.yaml`,
  mutations at 0.3 rps / burst 1) plus a 1.5–4s jitter floor — do not
  tight-loop writes back-to-back; the general daily budget (200) and the
  separate DM cap (10) both exit 2 when spent.
- Prefer `mute` over `block` when the goal is only to stop seeing someone:
  `block` is target-visible (the target can tell), `mute` is invisible.
- `list-add-member` pulls another user's content into your list without
  their consent — a documented spam-report vector. Add only on-topic
  accounts, one per invocation (no batch flag exists on purpose).
- `TWITTER_PROXY` for egress control; `--timeout/--max-retries` override config.
- Completions: `twr completions <bash|zsh|fish|powershell|elvish>` (script on
  stdout, install notes on stderr).

## 6. Direct messages — highest-scrutiny surface (read before `dm-send`)

Unsolicited automated DMs are among the most-cited platform-suspension
triggers across every reference surveyed for this tool's ban-risk research
(cookie-scraper repos, ban-risk writeups) — worse than mass-follow or
mass-like. A DM lands in a recipient's **private inbox**, not a public
timeline, so X's trust-and-safety systems treat it as a far stronger spam
signal than any timeline-visible action `twr` supports.

Rules (all enforced in code, not just guidance):

- `dm-send` requires `--policy write` **explicitly**. `--policy engagement`
  (which permits like/retweet/follow/bookmark) does NOT permit `dm-send` —
  DM's spam profile has nothing in common with those four actions, and the
  policy whitelist omits it deliberately. There is a dedicated test
  guarding against a future refactor accidentally widening engagement.
- `dm-send` has its **own daily cap** (`TWR_DM_DAILY_BUDGET`, default **10**,
  floor-clamped so it can never be disabled), independent of the general
  daily mutation budget (default 200). Exhausting either one exits 2.
- No batch flag exists and none will ever be added: one recipient per
  invocation. Looping/batching belongs in the orchestrator, not in `twr`.
  A future request for `--dm-file`/`--bulk-send` must be rejected or
  escalated to a human — never implemented quietly.
- Idempotency: retrying a send with the same key + same (recipient, text)
  never duplicates the message.

Recommendation: do NOT use `dm-send` for outbound-to-strangers traffic
(cold outreach, unsolicited offers) even under a human's direct instruction
— verify an authorized, solicited context first (replying to an inbound
message, an explicitly opted-in notification). Reads (`dm-list`,
`dm-read`) are lower-risk but DM content is user-private: it is never
logged above what a Tweet's text already is.

Wire status (honest): the `dm/new2.json` send body is triangulated from two
references (older `recipient_ids`-only shape vs twikit's
`conversation_id`-carrying `v11.dm_new`) with NO live solicited send yet
proving either — the code sends a documented superset with a discriminating
probe order (see bead `o1l.5.3`). Treat `dm-send` as wire-unverified until
that probe runs; `dm-list`/`dm-read` are live-verified reads.

## 7. List management (P6.3) — write-verification status

List reads (`lists`, `list-members`) are live-verified. List WRITES are
code-shipped but live-blocked: X answers 214 (DecodeException) on
`CreateList` despite variables matching two working references byte-for-byte
(Rettiwt-API + twikit) — suspected queryId-gated persisted-query rejection,
with twikit's newest ID queued as fallback (bead `o1l.3.1`). Until a live
create succeeds, treat every `list-create`/`list-edit`/`list-delete`/
`list-add-member`/`list-follow`/`list-pin` as unverified: dry-run the gate
freely (`--dry-run` proves policy/budget/idempotency), but do not script
against their `--apply` responses.

Tiers (enforced in `twr-core/src/policy.rs`, pinned by test
`p63_write_tier_stays_write_only_and_engagement_covers_list_engagement`):
`list-create`/`list-edit`/`list-delete` are write-only;
`list-follow`/`list-unfollow`/`list-pin`/`list-unpin`/`list-add-member`/
`list-remove-member` are engagement-tier.
