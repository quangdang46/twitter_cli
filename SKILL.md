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
- `TWITTER_PROXY` for egress control; `--timeout/--max-retries` override config.
- Completions: `twr completions <bash|zsh|fish|powershell|elvish>` (script on
  stdout, install notes on stderr).
