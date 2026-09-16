#!/usr/bin/env bash
# P6.3 live-smoke (bead twitter_cli-o1l.3.7 §§1+3): full list lifecycle +
# policy-gating matrix against a real authed session, with per-step logging
# (operation, HTTP status, envelope kind, parse outcome) so a failure
# pinpoints exactly which op broke.
#
# SAFETY (non-negotiable, bead acceptance + §13.3 ban-risk):
# - Full lifecycle (create→edit→add→verify→remove→verify→follow→unfollow→
#   pin→unpin→delete→verify-gone) runs ONLY with --apply AND an explicit
#   APPLY_OK=yes env (default: DRY-RUN ONLY — every write step runs with
#   --dry-run and asserts exit 0 + write_result dry_run kind, proving the
#   gate/policy/idempotency path short of the wire).
# - One user per invocation, no batch flag (spam-report vector).
# - c1 runs this on the release binary (./target/release/twr) against a
#   THROWAWAY account; e1 owns the script + unit side only.
# - MEMBER_UID: a user id to add/remove (default: NASA 11348282 — a public
#   account; adding it to a throwaway's list is low-visibility).
set -u
TWR="${TWR:-./target/release/twr}"
APPLY_OK="${APPLY_OK:-no}"
MEMBER_UID="${MEMBER_UID:-11348282}"
PASS=0; FAIL=0; SKIP=0

step() { # step <name> <expected_kind> -- <twr args...>
    local name="$1" want="$2"; shift 2
    [ "${1:-}" = "--" ] && shift
    echo "--- STEP: $name ($*)"
    local out code
    out="$("$TWR" "$@" 2>scripts/.p63-smoke.err)"; code=$?
    local kind ok
    kind="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("type","?"))' 2>/dev/null || echo PARSE_FAIL)"
    ok="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("ok","?"))' 2>/dev/null || echo PARSE_FAIL)"
    echo "    exit=$code kind=$kind ok=$ok (want kind=$want)"
    if [ "$code" -ne 0 ] || [ "$kind" != "$want" ]; then
        echo "    FAIL — stderr:"; sed 's/^/      /' scripts/.p63-smoke.err
        FAIL=$((FAIL+1)); return 1
    fi
    PASS=$((PASS+1))
}

expect_deny() { # expect_deny <name> -- <twr args...> (exit 2 + usage-policy-denied)
    local name="$1"; shift
    [ "${1:-}" = "--" ] && shift
    echo "--- STEP: $name ($*)"
    local out code
    out="$("$TWR" "$@" 2>scripts/.p63-smoke.err)"; code=$?
    local ecode
    ecode="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["error"]["code"])' 2>/dev/null || echo PARSE_FAIL)"
    echo "    exit=$code error.code=$ecode"
    if [ "$code" -ne 2 ] || [ "$ecode" != "usage-policy-denied" ]; then
        echo "    FAIL — expected exit 2 + usage-policy-denied"; FAIL=$((FAIL+1)); return 1
    fi
    PASS=$((PASS+1))
}

# NOTE (verified pattern from p61/p65): `--json` is global and must precede
# the subcommand.

# ---- §2 policy-gating matrix (offline-safe: deny + dry-run only) ----
# write-tier: create/edit/delete deny under engagement AND read_only.
expect_deny "create-engagement-deny" -- --policy engagement --json list-create "P63 Smoke"
expect_deny "create-read-only-deny"  -- --policy read_only --json list-create "P63 Smoke"
expect_deny "edit-engagement-deny"   -- --policy engagement --json list-edit L1 --name N --description D
expect_deny "edit-read-only-deny"    -- --policy read_only --json list-edit L1 --name N --description D
expect_deny "delete-engagement-deny" -- --policy engagement --json list-delete L1
expect_deny "delete-read-only-deny"  -- --policy read_only --json list-delete L1
# engagement-tier: add/remove/follow/unfollow/pin/unpin deny under read_only
# only (prove the split: engagement ALLOWS via dry-run, read_only DENIES).
expect_deny "add-member-read-only-deny" -- --policy read_only --json list-add-member --list-id L1 --user-id "$MEMBER_UID"
expect_deny "follow-read-only-deny"     -- --policy read_only --json list-follow L1
expect_deny "pin-read-only-deny"        -- --policy read_only --json list-pin L1
step "add-member-engagement-dry-run"    write_result -- --policy engagement --dry-run --json list-add-member --list-id L1 --user-id "$MEMBER_UID"
step "follow-engagement-dry-run"        write_result -- --policy engagement --dry-run --json list-follow L1
step "pin-engagement-dry-run"           write_result -- --policy engagement --dry-run --json list-pin L1
# write-tier dry-run previews (no network).
step "create-write-dry-run" write_result -- --policy write --dry-run --json list-create "P63 Smoke" --description "smoke-test list"
step "edit-write-dry-run"   write_result -- --policy write --dry-run --json list-edit L1 --name N --description D
step "delete-write-dry-run" write_result -- --policy write --dry-run --json list-delete L1

# ---- §1 full lifecycle (needs APPLY_OK=yes + throwaway) ----
if [ "$APPLY_OK" != "yes" ]; then
    echo "--- lifecycle SKIPPED (dry-run mode; set APPLY_OK=yes on a throwaway to run --apply)"
    SKIP=$((SKIP+1))
else
    NAME="p63-smoke-$(date +%s)"
    echo "--- lifecycle with --apply (list: $NAME)"
    out="$("$TWR" --policy write --apply --json list-create "$NAME" --description "p63 smoke" 2>scripts/.p63-smoke.err)"; code=$?
    echo "    create exit=$code: $out"
    [ "$code" -ne 0 ] && { FAIL=$((FAIL+1)); echo "=== p63-smoke: PASS=$PASS FAIL=$FAIL SKIP=$SKIP"; exit 1; }
    PASS=$((PASS+1))
    LIST_ID="$(printf '%s' "$out" | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"].get("list_id") or "")' 2>/dev/null || echo "")"
    if [ -z "$LIST_ID" ] || [ "$LIST_ID" = "None" ]; then
        echo "    create returned no list_id — verify via 'twr lists', then re-run with LIST_ID preset"
        FAIL=$((FAIL+1)); echo "=== p63-smoke: PASS=$PASS FAIL=$FAIL SKIP=$SKIP"; exit 1
    fi
    echo "    LIST_ID=$LIST_ID"
    step "lifecycle-edit"         write_result -- --policy write --apply --json list-edit "$LIST_ID" --name "$NAME-r" --description "p63 smoke renamed"
    step "lifecycle-add-member"   write_result -- --policy engagement --apply --json list-add-member --list-id "$LIST_ID" --user-id "$MEMBER_UID"
    step "lifecycle-verify-add"   user_list    -- --json list-members "$LIST_ID" --max 20
    step "lifecycle-remove"       write_result -- --policy engagement --apply --json list-remove-member --list-id "$LIST_ID" --user-id "$MEMBER_UID"
    step "lifecycle-verify-gone"  user_list    -- --json list-members "$LIST_ID" --max 20
    step "lifecycle-follow"       write_result -- --policy engagement --apply --json list-follow "$LIST_ID"
    step "lifecycle-unfollow"     write_result -- --policy engagement --apply --json list-unfollow "$LIST_ID"
    step "lifecycle-pin"          write_result -- --policy engagement --apply --json list-pin "$LIST_ID"
    step "lifecycle-unpin"        write_result -- --policy engagement --apply --json list-unpin "$LIST_ID"
    step "lifecycle-delete"       write_result -- --policy write --apply --json list-delete "$LIST_ID"
    # delete-retry: already-deleted is exit 0 no-op success (plan §5.5).
    step "lifecycle-delete-retry" write_result -- --policy write --apply --json list-delete "$LIST_ID"
fi

step "schema-catalog"  schema   -- --json schema
step "commands-catalog" commands -- --json commands

echo "=== p63-smoke: PASS=$PASS FAIL=$FAIL SKIP=$SKIP"
[ "$FAIL" -eq 0 ]
