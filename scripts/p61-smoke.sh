#!/usr/bin/env bash
# P6.1 live-smoke (bead twitter_cli-o1l.1.8 §2): exercises bookmarks
# --folders/--folder, mentions, notifications, user-replies, user-media,
# lists, and list-members in sequence against a real authed session, with
# per-step logging (operation, HTTP status, envelope kind, parse outcome)
# so a failure pinpoints exactly which op broke.
#
# READ-ONLY: no --apply anywhere (all seven are reads). Safe to run; exits
# non-zero on the first failing step. c1 runs this on the release binary
# (./target/release/twr); e1 owns the script + unit side only.
set -u
TWR="${TWR:-./target/release/twr}"
HANDLE="${HANDLE:-nasa}"
LIST_ID="${LIST_ID:-1970899750481260878}" # NASA '2025 Astronaut Candidates' (5 members, bead .1.7)
PASS=0; FAIL=0

step() { # step <name> <expected_kind> -- <twr args...>
    local name="$1" want="$2"; shift 2
    [ "${1:-}" = "--" ] && shift
    echo "--- STEP: $name ($*)"
    local out code
    out="$("$TWR" "$@" --json 2>scripts/.p61-smoke.err)"; code=$?
    local kind ok returned
    kind="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("type","?"))' 2>/dev/null || echo PARSE_FAIL)"
    ok="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("ok","?"))' 2>/dev/null || echo PARSE_FAIL)"
    returned="$(printf '%s' "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin).get("data",{}); p=d.get("page",{}); print(p.get("returned", d.get("returned","?")))' 2>/dev/null || echo ?)"
    echo "    exit=$code kind=$kind ok=$ok returned=$returned (want kind=$want)"
    if [ "$code" -ne 0 ] || [ "$kind" != "$want" ]; then
        echo "    FAIL — stderr:"; sed 's/^/      /' scripts/.p61-smoke.err
        FAIL=$((FAIL+1)); return 1
    fi
    PASS=$((PASS+1))
}

# NOTE (verified live 2026-09-16 on release binary): `lists` takes a
# POSITIONAL id (`lists 11348282`), not `--id`; all P6.1 commands need
# `--json` BEFORE the subcommand (global flag). `mentions` on an account
# with no mentions returns events:[] (Top-cursor rows are paging state,
# not events — parser fix in this same bead).
step "bookmarks-folders"  bookmark_folder_list -- --json bookmarks --folders
step "mentions"            notification_list    -- --json mentions --max 5
step "notifications"       notification_list    -- --json notifications --max 5
step "user-replies"        tweet_list           -- --json user-replies "$HANDLE" --max 5
step "user-media"          tweet_list           -- --json user-media "$HANDLE" --max 5
step "lists"               list_list            -- --json lists 11348282 --max 5
step "list-members"        user_list            -- --json list-members "$LIST_ID" --max 5
step "schema-catalog"      schema               -- --json schema
step "commands-catalog"    commands             -- --json commands

echo "=== p61-smoke: PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ]
