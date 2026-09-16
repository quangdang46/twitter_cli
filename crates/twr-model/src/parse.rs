//! GraphQL response -> domain model, ported from
//! `tmp/_research/twitter-py/twitter_cli/parser.py`. All the edge cases
//! `COMPREHENSIVEPLANFORTWITTERCLI.md` §1.2 calls out as needing fixture
//! tests are handled here: tombstone/visibility unwrapping, retweet
//! unwrapping, `note_tweet` full text, media (photo-original / best video
//! mp4), quote tweets, article extraction, the promoted flag.

use crate::article::parse_article;
use crate::deep_get::parse_int;
use crate::dget;
use crate::model::{
    Author, BookmarkFolder, DmConversation, DmMessage, ListOwner, Metrics, NotificationActor,
    NotificationEvent, Tweet, TweetMedia, TwitterList, UserProfile,
};
use serde_json::Value;

fn extract_media(legacy: &Value) -> Vec<TweetMedia> {
    let mut media = Vec::new();
    let Some(Value::Array(items)) = dget!(legacy, "extended_entities", "media") else {
        return media;
    };
    for item in items {
        let media_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let width = dget!(item, "original_info", "width").and_then(Value::as_i64);
        let height = dget!(item, "original_info", "height").and_then(Value::as_i64);
        match media_type {
            "photo" => {
                let url = item
                    .get("media_url_https")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                media.push(TweetMedia {
                    media_type: "photo".to_string(),
                    url,
                    width,
                    height,
                });
            }
            "video" | "animated_gif" => {
                let mut mp4_variants: Vec<&Value> = dget!(item, "video_info", "variants")
                    .and_then(Value::as_array)
                    .map(|v| {
                        v.iter()
                            .filter(|variant| {
                                variant.get("content_type").and_then(Value::as_str)
                                    == Some("video/mp4")
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Python: `sort(key=lambda v: v.get("bitrate", 0), reverse=True)`.
                mp4_variants.sort_by(|a, b| {
                    let ba = a.get("bitrate").and_then(Value::as_i64).unwrap_or(0);
                    let bb = b.get("bitrate").and_then(Value::as_i64).unwrap_or(0);
                    bb.cmp(&ba)
                });
                let url = mp4_variants
                    .first()
                    .and_then(|v| v.get("url"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        item.get("media_url_https")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string()
                    });
                media.push(TweetMedia {
                    media_type: media_type.to_string(),
                    url,
                    width,
                    height,
                });
            }
            _ => {}
        }
    }
    media
}

fn extract_author(user_data: &Value, user_legacy: &Value) -> Author {
    let empty = Value::Object(Default::default());
    let user_core = user_data.get("core").unwrap_or(&empty);
    let name = user_core
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| user_legacy.get("name").and_then(Value::as_str))
        .or_else(|| user_data.get("name").and_then(Value::as_str))
        .unwrap_or("Unknown")
        .to_string();
    let screen_name = user_core
        .get("screen_name")
        .and_then(Value::as_str)
        .or_else(|| user_legacy.get("screen_name").and_then(Value::as_str))
        .or_else(|| user_data.get("screen_name").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();
    let profile_image_url = dget!(user_data, "avatar", "image_url")
        .and_then(Value::as_str)
        .or_else(|| {
            user_legacy
                .get("profile_image_url_https")
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string();
    let verified = user_data
        .get("is_blue_verified")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || user_legacy
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    Author {
        id: user_data
            .get("rest_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name,
        screen_name,
        profile_image_url,
        verified,
    }
}

/// Unwrap `TweetWithVisibilityResults`, returning `(inner, is_subscriber_only)`.
fn unwrap_visibility(result: &Value) -> (&Value, bool) {
    if result.get("__typename").and_then(Value::as_str) == Some("TweetWithVisibilityResults") {
        if let Some(tweet) = result.get("tweet") {
            if !tweet.is_null() {
                return (
                    tweet,
                    result
                        .get("tweetInterstitial")
                        .map(|v| !v.is_null())
                        .unwrap_or(false),
                );
            }
        }
    }
    (result, false)
}

/// Parse a user result object into [`UserProfile`]. Mirrors `parse_user_result`.
pub fn parse_user_result(user_data: &Value) -> Option<UserProfile> {
    if user_data.get("__typename").and_then(Value::as_str) == Some("UserUnavailable") {
        return None;
    }
    let empty = Value::Object(Default::default());
    let legacy = user_data.get("legacy").unwrap_or(&empty);
    let core = user_data.get("core").unwrap_or(&empty);
    let avatar = user_data.get("avatar").unwrap_or(&empty);
    let location_obj = user_data.get("location").unwrap_or(&empty);

    let rest_id = user_data.get("rest_id").and_then(Value::as_str)?;
    if rest_id.is_empty() {
        return None;
    }

    Some(UserProfile {
        id: rest_id.to_string(),
        name: core
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| legacy.get("name").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        screen_name: core
            .get("screen_name")
            .and_then(Value::as_str)
            .or_else(|| legacy.get("screen_name").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        bio: legacy
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        location: location_obj
            .get("location")
            .and_then(Value::as_str)
            .or_else(|| legacy.get("location").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        url: dget!(legacy, "entities", "url", "urls", 0, "expanded_url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        followers_count: parse_int(legacy.get("followers_count"), 0),
        following_count: parse_int(legacy.get("friends_count"), 0),
        tweets_count: parse_int(legacy.get("statuses_count"), 0),
        likes_count: parse_int(legacy.get("favourites_count"), 0),
        verified: user_data
            .get("is_blue_verified")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || legacy
                .get("verified")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        profile_image_url: avatar
            .get("image_url")
            .and_then(Value::as_str)
            .or_else(|| {
                legacy
                    .get("profile_image_url_https")
                    .and_then(Value::as_str)
            })
            .unwrap_or_default()
            .to_string(),
        created_at: core
            .get("created_at")
            .and_then(Value::as_str)
            .or_else(|| legacy.get("created_at").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
    })
}

/// Parse a single `TweetResult` into a [`Tweet`]. Mirrors `parse_tweet_result`,
/// including its `depth` recursion guard (quote tweets can nest, but not
/// infinitely) and the retweet-unwrap-then-reparent-onto-actual_* dance.
pub fn parse_tweet_result(result: &Value, depth: u8) -> Option<Tweet> {
    if depth > 2 {
        return None;
    }
    let (tweet_data, is_subscriber_only) = unwrap_visibility(result);
    if tweet_data.get("__typename").and_then(Value::as_str) == Some("TweetTombstone") {
        return None;
    }

    // Presence-only checks, mirroring Python's `isinstance(legacy, dict) and isinstance(core, dict)` guard.
    if !tweet_data
        .get("legacy")
        .map(Value::is_object)
        .unwrap_or(false)
        || !tweet_data
            .get("core")
            .map(Value::is_object)
            .unwrap_or(false)
    {
        return None;
    }
    let legacy_v = tweet_data.get("legacy").unwrap();

    let user = dget!(tweet_data, "core", "user_results", "result")
        .cloned()
        .unwrap_or(Value::Null);
    let user_legacy = user.get("legacy").cloned().unwrap_or(Value::Null);
    let user_core = user.get("core").cloned().unwrap_or(Value::Null);

    let is_retweet = dget!(legacy_v, "retweeted_status_result", "result")
        .map(|v| !v.is_null())
        .unwrap_or(false);

    let mut retweet_subscriber_only = false;
    let fallback = || {
        (
            tweet_data.clone(),
            legacy_v.clone(),
            user.clone(),
            user_legacy.clone(),
        )
    };
    let (actual_data, actual_legacy, actual_user, actual_user_legacy): (
        Value,
        Value,
        Value,
        Value,
    ) = if is_retweet {
        let rt_result = dget!(legacy_v, "retweeted_status_result", "result")
            .cloned()
            .unwrap_or(Value::Null);
        let (rt_unwrapped, rt_sub) = unwrap_visibility(&rt_result);
        let rt_unwrapped = rt_unwrapped.clone();
        retweet_subscriber_only = rt_sub;
        let rt_legacy_is_object = rt_unwrapped
            .get("legacy")
            .map(Value::is_object)
            .unwrap_or(false);
        let rt_core_is_object = rt_unwrapped
            .get("core")
            .map(Value::is_object)
            .unwrap_or(false);
        if rt_legacy_is_object && rt_core_is_object {
            let rt_legacy = rt_unwrapped.get("legacy").cloned().unwrap();
            let rt_user = dget!(&rt_unwrapped, "core", "user_results", "result")
                .cloned()
                .unwrap_or(Value::Null);
            let rt_user_legacy = rt_user.get("legacy").cloned().unwrap_or(Value::Null);
            (rt_unwrapped.clone(), rt_legacy, rt_user, rt_user_legacy)
        } else {
            fallback()
        }
    } else {
        fallback()
    };

    let media = extract_media(&actual_legacy);
    let urls: Vec<String> = dget!(&actual_legacy, "entities", "urls")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|u| {
                    u.get("expanded_url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    let quoted = dget!(&actual_data, "quoted_status_result", "result").cloned();
    let quoted_tweet = quoted
        .filter(Value::is_object)
        .and_then(|q| parse_tweet_result(&q, depth + 1))
        .map(Box::new);
    let author = extract_author(&actual_user, &actual_user_legacy);

    let retweeted_by = if is_retweet {
        Some(
            user_core
                .get("screen_name")
                .and_then(Value::as_str)
                .or_else(|| user_legacy.get("screen_name").and_then(Value::as_str))
                .unwrap_or("unknown")
                .to_string(),
        )
    } else {
        None
    };

    let note_text = dget!(
        &actual_data,
        "note_tweet",
        "note_tweet_results",
        "result",
        "text"
    )
    .and_then(Value::as_str);
    let text = note_text
        .unwrap_or_else(|| {
            actual_legacy
                .get("full_text")
                .and_then(Value::as_str)
                .unwrap_or_default()
        })
        .to_string();

    let article = parse_article(&actual_data);

    let is_subscriber_only = if is_retweet {
        is_subscriber_only || retweet_subscriber_only
    } else {
        is_subscriber_only
    };

    Some(Tweet {
        id: actual_data
            .get("rest_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        text,
        author,
        metrics: Metrics {
            likes: parse_int(actual_legacy.get("favorite_count"), 0),
            retweets: parse_int(actual_legacy.get("retweet_count"), 0),
            replies: parse_int(actual_legacy.get("reply_count"), 0),
            quotes: parse_int(actual_legacy.get("quote_count"), 0),
            views: parse_int(dget!(&actual_data, "views", "count"), 0),
            bookmarks: parse_int(actual_legacy.get("bookmark_count"), 0),
        },
        created_at: actual_legacy
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        media,
        urls,
        is_retweet,
        lang: actual_legacy
            .get("lang")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        retweeted_by,
        quoted_tweet,
        score: None,
        article_title: article.title,
        article_text: article.text,
        is_subscriber_only,
        is_promoted: false,
    })
}

fn extract_cursor(content: &Value) -> Option<String> {
    if content.get("cursorType").and_then(Value::as_str) == Some("Bottom") {
        content
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string)
    } else {
        None
    }
}

/// Parse a `BookmarkFoldersSlice` response into `(folders, next_cursor)`.
/// Mirrors the Python `fetch_bookmark_folders` payload walk:
/// `data.viewer.user_results.result.bookmark_collections_slice` with
/// `items[]` (`{id, name}`) and `slice_info.next_cursor`. Skips items with
/// no `id` (same as Python's `if folder_id:` guard). Paginated by the
/// caller: pass the returned cursor back via `variables.cursor` until it
/// stops changing or goes missing, mirroring the Python `max_pages=10` loop.
pub fn parse_bookmark_folders_response(data: &Value) -> (Vec<BookmarkFolder>, Option<String>) {
    let empty_items: Vec<Value> = Vec::new();
    let slice = data
        .get("data")
        .and_then(|d| d.get("viewer"))
        .and_then(|v| v.get("user_results"))
        .and_then(|u| u.get("result"))
        .and_then(|r| r.get("bookmark_collections_slice"));
    let Some(slice) = slice else {
        return (Vec::new(), None);
    };
    let items = slice
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or(&empty_items);
    let mut folders = Vec::new();
    for item in items {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        folders.push(BookmarkFolder {
            id: id.to_string(),
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    let next = slice
        .get("slice_info")
        .and_then(|s| s.get("next_cursor"))
        .and_then(Value::as_str)
        .map(str::to_string);
    (folders, next)
}

/// Parse one `itemContent.list` result into a [`TwitterList`]. Mirrors
/// bird's `parseList` / xfetch's `parseList`: requires `id_str` + `name`
/// (returns None otherwise), reads `member_count`/`subscriber_count` as
/// ints-or-strings, and treats `mode == "private"` as private. The owner
/// comes from `user_results.result` (`rest_id` + legacy
/// `screen_name`/`name`) when present.
pub fn parse_list_result(list: &Value) -> Option<TwitterList> {
    let id = list.get("id_str").and_then(Value::as_str)?;
    let name = list.get("name").and_then(Value::as_str)?;
    let owner_result = list.get("user_results").and_then(|u| u.get("result"));
    let owner = owner_result.map(|o| {
        let legacy = o.get("legacy");
        ListOwner {
            id: o
                .get("rest_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            screen_name: legacy
                .and_then(|l| l.get("screen_name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            name: legacy
                .and_then(|l| l.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }
    });
    Some(TwitterList {
        id: id.to_string(),
        name: name.to_string(),
        description: list
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        member_count: parse_int(list.get("member_count"), 0),
        subscriber_count: parse_int(list.get("subscriber_count"), 0),
        is_private: list
            .get("mode")
            .and_then(Value::as_str)
            .is_some_and(|m| m.eq_ignore_ascii_case("private")),
        created_at: list
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        owner,
    })
}

/// Walk a `ListOwnerships`/`ListMemberships` response's instructions,
/// collecting every `content.itemContent.list` entry into [`TwitterList`]s
/// plus the `Bottom` cursor. Same timeline-instruction/cursor convention as
/// `parse_timeline_response` (bird's `parseListsFromInstructions` verbatim).
/// `get_instructions` is the same extractor style: pass a closure walking
/// `data.user.result.timeline.timeline.instructions`.
pub fn parse_lists_response(
    data: &Value,
    get_instructions: fn(&Value) -> Option<&Vec<Value>>,
) -> (Vec<TwitterList>, Option<String>) {
    let mut lists = Vec::new();
    let mut next_cursor = None;
    let Some(instructions) = get_instructions(data) else {
        return (lists, next_cursor);
    };
    for instruction in instructions {
        let entries: Vec<&Value> = instruction
            .get("entries")
            .and_then(Value::as_array)
            .map(|v| v.iter().collect())
            .unwrap_or_default();
        for entry in entries {
            let empty = Value::Object(Default::default());
            let content = entry.get("content").unwrap_or(&empty);
            if let Some(c) = extract_cursor(content) {
                next_cursor = Some(c);
            }
            if let Some(list) = content
                .get("itemContent")
                .and_then(|ic| ic.get("list"))
                .and_then(parse_list_result)
            {
                lists.push(list);
            }
        }
    }
    (lists, next_cursor)
}

/// Walk a `ListMembers` response's instructions (`data.list.
/// members_timeline.timeline.instructions`), collecting `content.
/// itemContent.user_results.result` entries via [`parse_user_result`] plus
/// the `Bottom` cursor. Members ARE users — no new model type (xfetch's
/// `parseListMembers` verbatim, minus its hand-rolled user mapping: we
/// reuse `parse_user_result` instead).
pub fn parse_list_members_response(
    data: &Value,
    get_instructions: fn(&Value) -> Option<&Vec<Value>>,
) -> (Vec<UserProfile>, Option<String>) {
    let mut members = Vec::new();
    let mut next_cursor = None;
    let Some(instructions) = get_instructions(data) else {
        return (members, next_cursor);
    };
    for instruction in instructions {
        let entries: Vec<&Value> = instruction
            .get("entries")
            .and_then(Value::as_array)
            .map(|v| v.iter().collect())
            .unwrap_or_default();
        for entry in entries {
            let empty = Value::Object(Default::default());
            let content = entry.get("content").unwrap_or(&empty);
            if let Some(c) = extract_cursor(content) {
                next_cursor = Some(c);
            }
            let member = content
                .get("itemContent")
                .and_then(|ic| ic.get("user_results"))
                .and_then(|ur| ur.get("result"))
                .and_then(parse_user_result);
            if let Some(profile) = member {
                members.push(profile);
            }
        }
    }
    (members, next_cursor)
}

/// Parse a timeline GraphQL response into `(tweets, next_cursor)`. Mirrors
/// `parse_timeline_response`, including the `entries`/`moduleItems` fallback
/// and the nested-`items` (module) traversal for e.g. "who to follow" slots
/// interleaved with tweets.
pub fn parse_timeline_response<'a, F>(
    data: &'a Value,
    get_instructions: F,
) -> (Vec<Tweet>, Option<String>)
where
    F: Fn(&'a Value) -> Option<&'a Vec<Value>>,
{
    let mut tweets = Vec::new();
    let mut next_cursor = None;

    let Some(instructions) = get_instructions(data) else {
        return (tweets, next_cursor);
    };

    for instruction in instructions {
        let entries: Vec<&Value> = instruction
            .get("entries")
            .and_then(Value::as_array)
            .or_else(|| instruction.get("moduleItems").and_then(Value::as_array))
            .map(|v| v.iter().collect())
            .unwrap_or_default();

        for entry in entries {
            let empty = Value::Object(Default::default());
            let content = entry.get("content").unwrap_or(&empty);
            if let Some(c) = extract_cursor(content) {
                next_cursor = Some(c);
            }

            let item_content = content.get("itemContent").unwrap_or(&empty);
            if let Some(result) = dget!(item_content, "tweet_results", "result") {
                if let Some(mut tweet) = parse_tweet_result(result, 0) {
                    let entry_id = entry
                        .get("entryId")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    tweet.is_promoted = entry_id.starts_with("promoted-")
                        || item_content
                            .get("promotedMetadata")
                            .map(|v| !v.is_null())
                            .unwrap_or(false);
                    tweets.push(tweet);
                }
            }

            if let Some(Value::Array(items)) = content.get("items") {
                for nested_item in items {
                    if let Some(nested_result) = dget!(
                        nested_item,
                        "item",
                        "itemContent",
                        "tweet_results",
                        "result"
                    ) {
                        if let Some(mut tweet) = parse_tweet_result(nested_result, 0) {
                            let nested_item_content = dget!(nested_item, "item", "itemContent");
                            let entry_id = nested_item
                                .get("entryId")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let promoted = nested_item_content
                                .and_then(|c| c.get("promotedMetadata"))
                                .map(|v| !v.is_null())
                                .unwrap_or(false);
                            tweet.is_promoted = entry_id.starts_with("promoted-") || promoted;
                            tweets.push(tweet);
                        }
                    }
                }
            }
        }
    }

    (tweets, next_cursor)
}

/// Parse a URT notifications REST response
/// (`GET /i/api/2/notifications/{all,mentions}.json`, xfetch's
/// `NotificationsMixin::parseNotificationResponse` ported to this repo's
/// model) into `(events, tweets, next_cursor)`.
///
/// Shape: `globalObjects.tweets` / `globalObjects.users` hold the bodies
/// keyed by id; `timeline.instructions[].addEntries.entries[]` hold the
/// events, each `entry.content` carrying the display fields. Cursor lives
/// at `entry.content.operation.cursor` (`cursorType == "Bottom"`), with a
/// `replaceEntry` fallback — NOT the GraphQL `content.cursorType` shape,
/// hence a dedicated parser rather than reusing `extract_cursor`.
///
/// Event rows reference tweets by id; the tweet BODIES come only from
/// `globalObjects` (a URT tweet is a legacy REST shape, not a GraphQL
/// `TweetResult` — do not run `parse_tweet_result` on it). The caller owns
/// the `(event.tweet_ids ↔ tweets)` join; the envelope ships both so an
/// agent can resolve without a second call.
pub fn parse_notifications_response(
    data: &Value,
) -> (Vec<NotificationEvent>, Vec<Tweet>, Option<String>) {
    let empty_obj = Value::Object(Default::default());
    let global = data.get("globalObjects").unwrap_or(&empty_obj);
    let global_tweets = global
        .get("tweets")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let global_users = global
        .get("users")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    // Minimal REST-tweet → Tweet lift: id/text/author/metrics/created_at
    // from the legacy extended-mode fields (full_text, user_id_str,
    // favorite_/retweet_/reply_/quote_/bookmark_count, created_at, lang).
    // Media/quote-nesting stay empty — the notification surface is a
    // pointer to the tweet, `twr tweet <id>` has the full body.
    let mut tweets = Vec::new();
    for (tweet_id, t) in &global_tweets {
        let user_id = t
            .get("user_id_str")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let u = global_users.get(user_id).cloned().unwrap_or(Value::Null);
        tweets.push(Tweet {
            id: tweet_id.clone(),
            text: t
                .get("full_text")
                .or_else(|| t.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            author: Author {
                id: user_id.to_string(),
                name: u
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                screen_name: u
                    .get("screen_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                profile_image_url: u
                    .get("profile_image_url_https")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                verified: u.get("verified").and_then(Value::as_bool).unwrap_or(false),
            },
            metrics: Metrics {
                likes: parse_int(t.get("favorite_count"), 0),
                retweets: parse_int(t.get("retweet_count"), 0),
                replies: parse_int(t.get("reply_count"), 0),
                quotes: parse_int(t.get("quote_count"), 0),
                views: 0,
                bookmarks: parse_int(t.get("bookmark_count"), 0),
            },
            created_at: t
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            media: vec![],
            urls: vec![],
            is_retweet: t.get("retweeted_status_id_str").is_some(),
            lang: t
                .get("lang")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            retweeted_by: None,
            quoted_tweet: None,
            score: None,
            article_title: None,
            article_text: None,
            is_subscriber_only: false,
            is_promoted: false,
        });
    }
    // Newest-first, mirroring xfetch's createdAt sort.
    tweets.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let mut events = Vec::new();
    let mut next_cursor = None;
    let instructions = data
        .get("timeline")
        .and_then(|t| t.get("instructions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for instruction in &instructions {
        // replaceEntry fallback for the Bottom cursor.
        if let Some(cursor) = instruction
            .get("replaceEntry")
            .and_then(|r| r.get("entry"))
            .and_then(|e| e.get("content"))
            .and_then(|c| c.get("operation"))
            .and_then(|o| o.get("cursor"))
            .filter(|c| c.get("cursorType").and_then(Value::as_str) == Some("Bottom"))
            .and_then(|c| c.get("value"))
            .and_then(Value::as_str)
        {
            next_cursor = Some(cursor.to_string());
        }
        let entries = instruction
            .get("addEntries")
            .and_then(|a| a.get("entries"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entry in &entries {
            let content = entry.get("content").unwrap_or(&empty_obj);
            // ANY cursor row (Bottom AND Top — live `mentions` returns a bare
            // Top-cursor entry on accounts with no mentions, bead o1l.1.8) is
            // paging state, never an event: Bottom advances next_cursor,
            // Top is skipped without emitting a "cursor-top-*" pseudo-event.
            if let Some(cursor_obj) = content.get("operation").and_then(|o| o.get("cursor")) {
                let ctype = cursor_obj
                    .get("cursorType")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if ctype == "Bottom" {
                    if let Some(cursor) = cursor_obj.get("value").and_then(Value::as_str) {
                        next_cursor = Some(cursor.to_string());
                    }
                }
                continue;
            }
            // Event rows: icon names the type; message/url/timestamp come
            // from the content; actor + tweet ids from the entry id lists
            // (fallback: entryId itself when the lists are absent).
            let icon = content
                .get("icon")
                .and_then(|i| i.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let entry_id = entry
                .get("entryId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let event_type = if icon.is_empty() {
                entry_id.to_string()
            } else {
                icon.strip_prefix("icon_").unwrap_or(icon).to_string()
            };
            let mut actor_ids: Vec<String> = content
                .get("fromUserIds")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if actor_ids.is_empty() {
                actor_ids = content
                    .get("from_user_id_str")
                    .and_then(Value::as_str)
                    .map(|s| vec![s.to_string()])
                    .unwrap_or_default();
            }
            let actors: Vec<NotificationActor> = actor_ids
                .into_iter()
                .map(|id| {
                    let u = global_users.get(&id).cloned().unwrap_or(Value::Null);
                    NotificationActor {
                        id: id.clone(),
                        screen_name: u
                            .get("screen_name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: u
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    }
                })
                .collect();
            let mut tweet_ids: Vec<String> = content
                .get("tweetIds")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if tweet_ids.is_empty() {
                tweet_ids = content
                    .get("target_tweet_id_str")
                    .and_then(Value::as_str)
                    .map(|s| vec![s.to_string()])
                    .unwrap_or_default();
            }
            events.push(NotificationEvent {
                event_type,
                actors,
                tweet_ids,
                timestamp_ms: content
                    .get("timestampMs")
                    .and_then(Value::as_str)
                    .or_else(|| content.get("timestamp_ms").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string(),
                message: content
                    .get("message")
                    .and_then(|m| m.get("text").or(Some(m)))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }

    (events, tweets, next_cursor)
}

/// Parse a DM inbox_initial_state / inbox_timeline REST response into
/// conversations, UNCONFIRMED envelope (no fixture in the corpus — see
/// bead o1l.5.2 comment; Rettiwt's DirectMessage.ts covers request
/// construction, not response shape). Tolerant/best-effort: any of
/// `conversations` (object keyed by id) or `inbox.conversations` object.
pub fn parse_dm_inbox_response(data: &Value) -> (Vec<DmConversation>, Option<String>) {
    let empty_obj = serde_json::Map::new();
    let convs = data
        .get("conversations")
        .and_then(Value::as_object)
        .or_else(|| {
            data.pointer("/inbox_initial_state/conversations")
                .and_then(Value::as_object)
        })
        .unwrap_or(&empty_obj);
    let mut out = Vec::new();
    for (id, c) in convs {
        let participants = c
            .get("participants")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let uid = p.get("user_id").and_then(Value::as_str)?;
                        Some(NotificationActor {
                            id: uid.to_string(),
                            screen_name: String::new(),
                            name: String::new(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(DmConversation {
            id: id.clone(),
            participants,
            last_message: None,
            last_timestamp_ms: c
                .get("sort_timestamp")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            unread_count: 0,
        });
    }
    let next = data
        .pointer("/inbox_timeline/min_entry_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    (out, next)
}

/// Parse a DM conversation history response (`dm/conversation/<id>.json`)
/// into messages, UNCONFIRMED envelope (same caveat as
/// `parse_dm_inbox_response`). Tolerant: `entries[].message.message_data`.
pub fn parse_dm_conversation_response(data: &Value) -> (Vec<DmMessage>, Option<String>) {
    let empty = Vec::new();
    let entries = data
        .pointer("/conversation_timeline/entries")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut out = Vec::new();
    for e in entries {
        let Some(md) = e.pointer("/message/message_data") else {
            continue;
        };
        let Some(id) = md.get("id").and_then(Value::as_str) else {
            continue;
        };
        out.push(DmMessage {
            id: id.to_string(),
            sender_id: md
                .get("sender_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            text: md
                .pointer("/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            timestamp_ms: md
                .get("time")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    let next = data
        .pointer("/conversation_timeline/min_entry_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    (out, next)
}
