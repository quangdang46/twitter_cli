//! `twr-model` — Tweet/User/Media structs plus the GraphQL response parser.
//!
//! Ported from `tmp/_research/twitter-py/twitter_cli/{models,parser}.py`
//! (see COMPREHENSIVEPLANFORTWITTERCLI.md §1.2/§7). `deep_get`'s `dget!`
//! macro is exported at the crate root because Rust macros aren't
//! automatically visible via `mod` the way Python's imports are.

pub mod article;
pub mod deep_get;
pub mod model;
pub mod parse;

pub use model::{Author, BookmarkFolder, Metrics, Tweet, TweetMedia, UserProfile};
pub use parse::{
    parse_bookmark_folders_response, parse_timeline_response, parse_tweet_result, parse_user_result,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal but structurally faithful `TweetResult`, matching the real
    /// GraphQL shape documented in `parser.py` (core/legacy split, per
    /// COMPREHENSIVEPLANFORTWITTERCLI.md §1.2's parser edge-case list).
    fn plain_tweet_result(id: &str, text: &str) -> serde_json::Value {
        json!({
            "rest_id": id,
            "core": {
                "user_results": {
                    "result": {
                        "rest_id": "u1",
                        "core": { "name": "Ada Lovelace", "screen_name": "ada" },
                        "legacy": {},
                        "is_blue_verified": true
                    }
                }
            },
            "legacy": {
                "full_text": text,
                "favorite_count": 10,
                "retweet_count": 2,
                "reply_count": 1,
                "quote_count": 0,
                "bookmark_count": 3,
                "created_at": "Mon Jan 01 00:00:00 +0000 2024",
                "lang": "en",
                "entities": { "urls": [] }
            },
            "views": { "count": "1,234" }
        })
    }

    #[test]
    fn parses_a_plain_tweet() {
        let result = plain_tweet_result("100", "hello world");
        let tweet = parse_tweet_result(&result, 0).expect("should parse");
        assert_eq!(tweet.id, "100");
        assert_eq!(tweet.text, "hello world");
        assert_eq!(tweet.author.screen_name, "ada");
        assert!(tweet.author.verified);
        assert_eq!(tweet.metrics.likes, 10);
        assert_eq!(
            tweet.metrics.views, 1234,
            "parse_int must strip commas from view counts"
        );
        assert!(!tweet.is_retweet);
        assert!(tweet.quoted_tweet.is_none());
    }

    #[test]
    fn tombstoned_tweets_return_none() {
        let result = json!({ "__typename": "TweetTombstone" });
        assert!(parse_tweet_result(&result, 0).is_none());
    }

    #[test]
    fn unwraps_tweet_with_visibility_results() {
        let inner = plain_tweet_result("200", "gated content");
        let wrapped = json!({
            "__typename": "TweetWithVisibilityResults",
            "tweet": inner,
            "tweetInterstitial": { "text": { "text": "Subscribers only" } }
        });
        let tweet = parse_tweet_result(&wrapped, 0).expect("should unwrap and parse");
        assert_eq!(tweet.id, "200");
        assert!(tweet.is_subscriber_only);
    }

    #[test]
    fn prefers_note_tweet_full_text_over_legacy_full_text() {
        let mut result = plain_tweet_result("300", "truncated…");
        result["note_tweet"] = json!({
            "note_tweet_results": { "result": { "text": "the real, much longer tweet body" } }
        });
        let tweet = parse_tweet_result(&result, 0).unwrap();
        assert_eq!(tweet.text, "the real, much longer tweet body");
    }

    #[test]
    fn unwraps_a_retweet_onto_the_original_authors_data() {
        let original = plain_tweet_result("400", "original text");
        let mut retweet_shell = plain_tweet_result("999", "RT @ada: original text");
        // The retweeter's own author (not ada) goes on the outer shell.
        retweet_shell["core"]["user_results"]["result"]["core"]["screen_name"] = json!("retweeter");
        retweet_shell["core"]["user_results"]["result"]["core"]["name"] = json!("Retweeter");
        retweet_shell["legacy"]["retweeted_status_result"] = json!({ "result": original });

        let tweet = parse_tweet_result(&retweet_shell, 0).unwrap();
        assert!(tweet.is_retweet);
        assert_eq!(
            tweet.id, "400",
            "id should come from the ORIGINAL tweet, not the retweet shell"
        );
        assert_eq!(tweet.text, "original text");
        assert_eq!(
            tweet.author.screen_name, "ada",
            "author should be the original author, not the retweeter"
        );
        assert_eq!(tweet.retweeted_by.as_deref(), Some("retweeter"));
    }

    #[test]
    fn parses_a_quote_tweet_recursively() {
        let quoted = plain_tweet_result("500", "the quoted tweet");
        let mut quoting = plain_tweet_result("501", "check this out");
        quoting["quoted_status_result"] = json!({ "result": quoted });

        let tweet = parse_tweet_result(&quoting, 0).unwrap();
        let inner = tweet.quoted_tweet.expect("should have a quoted tweet");
        assert_eq!(inner.id, "500");
        assert_eq!(inner.text, "the quoted tweet");
    }

    #[test]
    fn recursion_guard_stops_after_depth_two() {
        let inner = plain_tweet_result("1", "innermost");
        assert!(
            parse_tweet_result(&inner, 3).is_none(),
            "depth > 2 must bail out per the Python guard"
        );
    }

    #[test]
    fn extracts_photo_and_best_bitrate_video_media() {
        let mut result = plain_tweet_result("600", "media tweet");
        result["legacy"]["extended_entities"] = json!({
            "media": [
                { "type": "photo", "media_url_https": "https://pbs.twimg.com/photo.jpg", "original_info": {"width": 100, "height": 50} },
                { "type": "video", "media_url_https": "https://pbs.twimg.com/thumb.jpg", "video_info": { "variants": [
                    { "content_type": "video/mp4", "bitrate": 500, "url": "https://video.mp4/low" },
                    { "content_type": "video/mp4", "bitrate": 2000, "url": "https://video.mp4/high" },
                    { "content_type": "application/x-mpegURL", "url": "https://video.m3u8" }
                ]}}
            ]
        });
        let tweet = parse_tweet_result(&result, 0).unwrap();
        assert_eq!(tweet.media.len(), 2);
        assert_eq!(tweet.media[0].media_type, "photo");
        assert_eq!(tweet.media[0].width, Some(100));
        assert_eq!(tweet.media[1].media_type, "video");
        assert_eq!(
            tweet.media[1].url, "https://video.mp4/high",
            "must pick the highest-bitrate mp4 variant"
        );
    }

    #[test]
    fn parses_a_user_profile() {
        let user = json!({
            "rest_id": "u42",
            "core": { "name": "Grace Hopper", "screen_name": "grace", "created_at": "..." },
            "legacy": {
                "description": "Compiler pioneer",
                "followers_count": "1,000",
                "friends_count": 10,
                "statuses_count": 500,
                "favourites_count": 20,
                "entities": { "url": { "urls": [{ "expanded_url": "https://example.com" }] } }
            },
            "avatar": { "image_url": "https://pbs.twimg.com/avatar.jpg" },
            "is_blue_verified": true
        });
        let profile = parse_user_result(&user).expect("should parse");
        assert_eq!(profile.screen_name, "grace");
        assert_eq!(profile.followers_count, 1000, "parse_int must strip commas");
        assert_eq!(profile.url, "https://example.com");
        assert!(profile.verified);
    }

    #[test]
    fn unavailable_users_return_none() {
        let user = json!({ "__typename": "UserUnavailable" });
        assert!(parse_user_result(&user).is_none());
    }

    #[test]
    fn parses_bookmark_folders_slice_with_cursor() {
        let data = json!({
            "data": { "viewer": { "user_results": { "result": {
                "bookmark_collections_slice": {
                    "items": [
                        { "id": "f1", "name": "Reading" },
                        { "id": "f2", "name": "Research" },
                        { "name": "no id — skipped" }
                    ],
                    "slice_info": { "next_cursor": "NEXT" }
                }
            }}}}
        });
        let (folders, cursor) = parse_bookmark_folders_response(&data);
        assert_eq!(folders.len(), 2, "items without id must be skipped");
        assert_eq!(folders[0].id, "f1");
        assert_eq!(folders[0].name, "Reading");
        assert_eq!(folders[1].id, "f2");
        assert_eq!(cursor.as_deref(), Some("NEXT"));
    }

    #[test]
    fn bookmark_folders_slice_missing_shape_is_empty_not_an_error() {
        let (folders, cursor) = parse_bookmark_folders_response(&json!({"data": {}}));
        assert!(folders.is_empty());
        assert!(cursor.is_none());
    }

    #[test]
    fn parses_a_timeline_with_cursor_and_promoted_flag() {
        let data = json!({
            "instructions": [{
                "entries": [
                    {
                        "entryId": "tweet-100",
                        "content": { "itemContent": { "tweet_results": { "result": plain_tweet_result("100", "first") } } }
                    },
                    {
                        "entryId": "promoted-200",
                        "content": { "itemContent": { "tweet_results": { "result": plain_tweet_result("200", "sponsored") } } }
                    },
                    {
                        "entryId": "cursor-bottom-0",
                        "content": { "cursorType": "Bottom", "value": "NEXT_CURSOR_VALUE" }
                    }
                ]
            }]
        });
        let (tweets, cursor) =
            parse_timeline_response(&data, |d| d.get("instructions").and_then(|v| v.as_array()));
        assert_eq!(tweets.len(), 2);
        assert!(!tweets[0].is_promoted);
        assert!(tweets[1].is_promoted);
        assert_eq!(cursor.as_deref(), Some("NEXT_CURSOR_VALUE"));
    }
}
