#!/usr/bin/env bash
# P6.5 live-smoke (bead twitter_cli-o1l.5.4 §§1+4): exercises dm-list,
# dm-read, and the dm-send POLICY/CAP gates in sequence against a real
# authed session, with per-step logging (operation, HTTP status, envelope
# kind, parse outcome) so a failure pinpoints exactly which op broke.
#
# SAFETY (non-negotiable, bead §1 + SKILL.md §6):
# - NO live --apply anywhere in this script. dm-send is covered by its
#   policy-denial + dry-run previews only (engagement deny, read_only deny,
#   write+dry-run ok) — a real send needs TWO throwaway accounts under the
#   operator's own control + explicit user approval, arranged by c1 at
#   close time. This script proves everything UP TO the send wire.
# - dm-read runs only when dm-list returns a conversation (else SKIP, not
#   FAIL — the test account may have zero DM history).
# - c1 runs this on the release binary (./target/release/twr); e1 owns the
#   script + unit side only.
set -u
TWR="${TWR:-./target/release/twr}"
PASS=0; FAIL=0; SKIP=0

step() { # step <name> <expected_kind> -- <twr args...>
    local name="$1" want="$2"; shift 2
    [ "${1:-}" = "--" ] && shift
    echo "--- STEP: $name ($*)"
    local out code
    out="$("$TWR" "$@" 2>scripts/.p65-smoke.err)"; code=$?
    local kind ok returned
    kind="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("type","?"))' 2>/dev/null || echo PARSE_FAIL)"
    ok="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("ok","?"))' 2>/dev/null || echo PARSE_FAIL)"
    returned="$(printf '%s' "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin).get("data",{}); p=d.get("page",{}); print(p.get("returned", d.get("returned","?")))' 2>/dev/null || echo ?)"
    echo "    exit=$code kind=$kind ok=$ok returned=$returned (want kind=$want)"
    if [ "$code" -ne 0 ] || [ "$kind" != "$want" ]; then
        echo "    FAIL — stderr:"; sed 's/^/      /' scripts/.p65-smoke.err
        FAIL=$((FAIL+1)); return 1
    fi
    PASS=$((PASS+1))
}

expect_deny() { # expect_deny <name> -- <twr args...> (exit 2 + usage-policy-denied)
    local name="$1"; shift
    [ "${1:-}" = "--" ] && shift
    echo "--- STEP: $name ($*)"
    local out code
    out="$("$TWR" "$@" 2>scripts/.p65-smoke.err)"; code=$?
    local ecode msg
    ecode="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["error"]["code"])' 2>/dev/null || echo PARSE_FAIL)"
    msg="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["error"]["message"])' 2>/dev/null || echo PARSE_FAIL)"
    echo "    exit=$code error.code=$ecode msg=$msg"
    if [ "$code" -ne 2 ] || [ "$ecode" != "usage-policy-denied" ]; then
        echo "    FAIL — expected exit 2 + usage-policy-denied"; FAIL=$((FAIL+1)); return 1
    fi
    PASS=$((PASS+1))
}

# NOTE (verified live 2026-09-16): `--json` is global and must precede the
# subcommand; dm-send takes <USER_ID> <TEXT> positionals.
step "dm-list"           dm_list      -- --json dm-list
expect_deny "dm-send-engagement-deny" -- --policy engagement --json dm-send 999 probe
expect_deny "dm-send-read-only-deny"  -- --policy read_only --json dm-send 999 probe
step "dm-send-write-dry-run" write_result -- --policy write --dry-run --json dm-send 999 hello-probe

# dm-read only when a conversation exists; else SKIP (not FAIL).
CONV="$(./target/release/twr --json dm-list 2>/dev/null | python3 -c 'import json,sys; d=json.load(sys.stdin)["data"].get("conversations",[]); print(d[0]["id"] if d else "")')"
if [ -n "$CONV" ]; then
    step "dm-read" dm_message_list -- --json dm-read "$CONV"
else
    echo "--- STEP: dm-read SKIPPED (no conversations on this account)"
    SKIP=$((SKIP+1))
fi

step "schema-catalog"  schema   -- --json schema
step "commands-catalog" commands -- --json commands

echo "=== p65-smoke: PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
[ "$FAIL" -eq 0 ]
