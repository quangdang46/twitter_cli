# twr — Envelope Schema (v1)

Every machine invocation prints exactly one document on stdout
(`{"ok":true,...}` or `{"ok":false,...}`); diagnostics go to stderr.
`schema_version` is always `"1"`. `ok` is a real boolean.

## Success

```json
{
  "ok": true,
  "schema_version": "1",
  "type": "tweet_list|user_list|user|tweet_detail|article|status|auth|write_result|schema|commands|doctor|query-ids|dm_list|dm_message_list|notification_list|list_list|headline_list|bookmark_folder_list|watchlist|error",
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
    "retry_after_ms": 900000,
    "failing_input": {"flag": "--max", "value": "500"},
    "outcome": "rejected|unknown|not_sent"
  },
  "meta": {"traceId": "…"}
}
```

`outcome` is present only on write-command failures (null elsewhere) and says
what happened to the mutation:

- `rejected` — X answered and refused the write; the mutation did not apply.
- `unknown` — the request may have reached X (transport failure after send,
  HTTP 5xx, or a 2xx with no confirmation). Do NOT auto-retry; check whether
  the write landed first. `retryable` is always `false` for `unknown`.
- `not_sent` — a local refusal or failure known not to have reached X
  (policy denial, budget cap, stale query id, pre-send persistence failure).

Secret-flag values (`--cookie/--auth-token/--ct0/--proxy…`) are always
`"[REDACTED]"` in `failing_input` — enforced at construction.

## Types

- `tweet_list`: `{"tweets": Tweet[], "page": {...}}` (also used by `bookmarks --folder <id>`
  and `cache-search`, which adds a top-level `"query"` echo and emits cache-partial
  Tweet rows — `id/text/author.screen_name/created_at/metrics` only)
- `watchlist`: `{"watchlist": string[]}` from `watch list`; gated `watch add/remove`
  return `{"watched"|"unwatched": true, "handle": str}` on `--apply`, or the shared
  dry-run shape `{"dry_run": true, "operation": "watch-add|watch-remove",
  "validation": "passed", "handle": str, "local_only": true}` on `--dry-run`
- `future`: emits the INNER command's envelope type (`tweet_list`/`tweet_detail`/`user`) —
  `twr future -d <secs> feed|search|user|tweet` sleeps once, then really runs the read
- `completions`/`mcp` are the only commands with NO envelope (`type: "none"` in
  `twr commands`): completions prints a shell script, mcp serves JSON-RPC stdio
- `bookmark_folder_list`: `{"folders": [{id, name}], "page": {"returned": n}}` (from `bookmarks --folders`)
- `list_list`: `{"lists": TwitterList[], "page": {...}}` (from `lists [--member-of]`); `TwitterList{id,name,description,member_count,subscriber_count,is_private,created_at,owner?{id,screen_name,name}}`
- `list-members` reuses the `user_list` shape verbatim (`{"users": [...], "page": {...}}`)
- `notification_list`: `{"events": NotificationEvent[], "tweets": Tweet[], "page": {...}}` (from `mentions`/`notifications`); `NotificationEvent{event_type, actors[{id,screen_name,name}], tweet_ids[], timestamp_ms, message?}` — events are NOT Tweets, the `tweets` array holds the referenced bodies (globalObjects-resolved) so an agent needs no second call
- `mentions` vs `notifications`: SAME REST endpoint family (`/i/api/2/notifications/{mentions,all}.json`), server-side kind filter — `mentions` is not a client-side filter over `all`, and neither is `UserTweetsAndReplies`-on-self (that returns your own posts+replies, not others' mentions of you)
- `tweet_detail`: a single `Tweet`; `tweet --conversation` returns the whole parsed
  TweetDetail page instead: `{"tweets": Tweet[], "page": {"nextCursor"?, "hasMore"}}`
  (focal tweet included; pass `page.nextCursor` back as `--cursor` to continue)
- `article`: `{"title": str, "markdown": str}` (with `--markdown`)
- `user` / `user_list`: `UserProfile` / `{"users": [...], "page": {...}}` (list parsing
  completes with fixture parity). `user`, `user-posts`, `user-replies`,
  `user-media`, and `likes` accept a numeric user id **or a screen name**
  (ids go through `UserByRestId`, names through `UserByScreenName`; timeline
  commands skip the profile lookup entirely for numeric ids). `followers`/
  `following` take a numeric user id **or a
  screen name** (resolved through `UserByScreenName`); an argument that is neither exits 2,
  a name that resolves to nobody exits 3. `page` carries `returned`, `maxRequested`,
  `truncated` (partial data from a rate limit, as elsewhere) and `nextCursor`/`hasMore`
  when more exist — pass `nextCursor` back as `--cursor` to continue
- `status`: `{"authenticated": bool, "source": "flags|env|file|browser"|null, ...}`
- `auth`: login/logout results (`{"saved": bool, ...}`)
- `write_result`: `{"id"|"ok"|"dry_run": ...}` or `{"error": ...}`. `post`/`quote`
  add `"graphql_operation": "CreateTweet"|"CreateNoteTweet"` (auto-routed at
  the 280-weighted-char threshold — reported in `--dry-run` previews too, so
  an agent can confirm which mutation would run before `--apply`).
- `dm_list`: `{"conversations": DmConversation[], "page": {...}}` (from `dm-list`); `DmConversation{id, participants[{id,screen_name,name}], last_message?, last_timestamp_ms, unread_count}` — private; never logged beyond envelope
- `dm_message_list`: `{"conversation_id": str, "messages": DmMessage[], "page": {...}}` (from `dm-read`); `DmMessage{id, sender_id, text, timestamp_ms}` — same privacy note
- `doctor`: `{"checks": [{"check": "AUTH|CONFIG|QUERY_ID|TX_ID|TLS", "status": "pass|warn|fail", "suggestion": …}]}`
- `query-ids`: `{"operations": [{"operation": …, "query_id": …, "source": …}]}`
- `schema` / `commands`: this catalog, machine-readable

## Tweet / UserProfile

Field names mirror the Python original for fixture parity:
`Tweet{id,text,author{name,screen_name,profile_image_url,verified,
following?,followed_by?},
metrics{likes,retweets,replies,quotes,views,bookmarks},created_at,
media[{type,url,width,height,id?,media_key?,poster_url?,alt_text?,
duration_millis?,variants?[{url,content_type?,bitrate?}]}],urls,is_retweet,
lang,retweeted_by,
quoted_tweet,score,article_title,article_text,is_subscriber_only,is_promoted,
conversation_id?,in_reply_to_status_id?,
grok_translated_post_with_availability?,
possibly_sensitive?,display_text_range?,mentions?[{id,screen_name?,name?,
start?,end?}],in_reply_to_user_id?,in_reply_to_screen_name?,edit_control?,
favorited?,retweeted?,bookmarked?,has_birdwatch_notes?}` — the grok field
is the raw X `grok_translated_post_with_availability` object, present only
when the server supplies it (tweet-read ops send the
`responsive_web_grok_show_grok_translated_post` feature flag; issue #7).
All `?` fields are emitted only when X supplies them; explicit `false`
booleans are preserved (issues #13/#14). `poster_url` is the video/GIF
poster frame (`media_url_https`), `variants` keeps every playback variant
(mp4 + HLS) in supplied order, and `mentions` carries structured
`user_mentions` with text offsets. `has_birdwatch_notes` is a presence
indicator only — note contents are never fetched.
`UserProfile` gains the same optional `following?`/`followed_by?` viewer
flags (issue #14).
`is_promoted` is derived from timeline entry IDs/`promotedMetadata` only —
a standalone tweet read always reports `false` and cannot distinguish an
unpromoted tweet from an ad that arrived without timeline context.
`--compact` strips `profile_image_url`, media dims, `urls`.
