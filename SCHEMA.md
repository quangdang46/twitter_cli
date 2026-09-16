# twr — Envelope Schema (v1)

Every machine invocation prints exactly one document on stdout
(`{"ok":true,...}` or `{"ok":false,...}`); diagnostics go to stderr.
`schema_version` is always `"1"`. `ok` is a real boolean.

## Success

```json
{
  "ok": true,
  "schema_version": "1",
  "type": "tweet_list|user_list|user|tweet_detail|article|status|auth|write_result|schema|commands|doctor|query-ids|dry_run",
  "data": {},
  "pagination": {"nextCursor": "…", "hasMore": true},
  "meta": {"traceId": "…", "command": "search", "maxRequested": 20, "returned": 20, "filterApplied": false}
}
```

`pagination` appears on list types; `meta.filterApplied` only when
`--filter` was passed; `meta.truncated=true` ships with partial data on
rate-limit resume (exit 4).

## Error

```json
{
  "ok": false,
  "schema_version": "1",
  "type": "error",
  "error": {
    "code": "general-auth|usage-policy-denied|not-found|forbidden-rate-limited|network|contract-drift|attachment-io|auth-required",
    "message": "…",
    "suggestion": "…",
    "retryable": true,
    "retryAfterMs": 900000,
    "failingInput": {"flag": "--max", "value": "500"}
  },
  "meta": {"traceId": "…"}
}
```

Secret-flag values (`--cookie/--auth-token/--ct0/--proxy…`) are always
`"[REDACTED]"` in `failingInput` — enforced at construction.

## Types

- `tweet_list`: `{"tweets": Tweet[], "page": {...}}` (also used by `bookmarks --folder <id>`)
- `bookmark_folder_list`: `{"folders": [{id, name}], "page": {"returned": n}}` (from `bookmarks --folders`)
- `tweet_detail`: a single `Tweet`
- `article`: `{"title": str, "markdown": str}` (with `--markdown`)
- `user` / `user_list`: `UserProfile` / `{"users": [...]}` (list parsing
  completes with fixture parity)
- `status`: `{"authenticated": bool, "source": "flags|env|file|browser"|null, ...}`
- `auth`: login/logout results (`{"saved": bool, ...}`)
- `write_result`: `{"id"|"ok"|"dry_run": ...}` or `{"error": ...}`
- `doctor`: `{"checks": [{"check": "AUTH|CONFIG|QUERY_ID|TX_ID|TLS", "status": "pass|warn|fail", "suggestion": …}]}`
- `query-ids`: `{"operations": [{"operation": …, "query_id": …, "source": …}]}`
- `schema` / `commands`: this catalog, machine-readable

## Tweet / UserProfile

Field names mirror the Python original for fixture parity:
`Tweet{id,text,author{name,screen_name,profile_image_url,verified},
metrics{likes,retweets,replies,quotes,views,bookmarks},created_at,
media[{type,url,width,height}],urls,is_retweet,lang,retweeted_by,
quoted_tweet,score,article_title,article_text,is_subscriber_only,is_promoted}`.
`--compact` strips `profile_image_url`, media dims, `urls`.
