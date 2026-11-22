//! Issue #13 (media previews/metadata) + #14 (viewer follow state):
//! supplied vs absent metadata, explicit `false` flags, animated GIFs,
//! quoted/retweeted media, and present/absent `following`/`followed_by`.

use twr_model::{parse_tweet_result, parse_user_result};

fn tweet_result(legacy: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "rest_id": "t1",
        "core": {"user_results": {"result": {
            "rest_id": "u1",
            "core": {"name": "Ada", "screen_name": "ada"},
            "legacy": {},
        }}},
        "legacy": legacy,
    })
}

#[test]
fn video_media_preserves_poster_alt_duration_variants_and_ids() {
    let result = tweet_result(serde_json::json!({
        "full_text": "clip",
        "created_at": "t",
        "lang": "en",
        "extended_entities": {"media": [{
            "id_str": "1234567890",
            "media_key": "13_1234567890",
            "type": "video",
            "media_url_https": "https://img/poster.jpg",
            "ext_alt_text": "a cat playing piano",
            "original_info": {"width": 640, "height": 360},
            "video_info": {
                "duration_millis": 15000,
                "variants": [
                    {"content_type": "application/x-mpegURL", "url": "https://v/pl.m3u8"},
                    {"content_type": "video/mp4", "bitrate": 256000, "url": "https://v/lo.mp4"},
                    {"content_type": "video/mp4", "bitrate": 832000, "url": "https://v/hi.mp4"},
                ]
            }
        }]}
    }));
    let tweet = parse_tweet_result(&result, 0).unwrap();
    assert_eq!(tweet.media.len(), 1);
    let m = &tweet.media[0];
    assert_eq!(m.media_type, "video");
    // Playable url still picks the highest-bitrate mp4.
    assert_eq!(m.url, "https://v/hi.mp4");
    assert_eq!(m.id.as_deref(), Some("1234567890"));
    assert_eq!(m.media_key.as_deref(), Some("13_1234567890"));
    assert_eq!(m.poster_url.as_deref(), Some("https://img/poster.jpg"));
    assert_eq!(m.alt_text.as_deref(), Some("a cat playing piano"));
    assert_eq!(m.duration_millis, Some(15000));
    let variants = m.variants.as_ref().unwrap();
    assert_eq!(variants.len(), 3);
    // Supplied order preserved, HLS included.
    assert_eq!(
        variants[0].content_type.as_deref(),
        Some("application/x-mpegURL")
    );
    assert_eq!(variants[2].bitrate, Some(832000));
}

#[test]
fn animated_gif_gets_poster_and_variants() {
    let result = tweet_result(serde_json::json!({
        "full_text": "gif",
        "created_at": "t",
        "lang": "en",
        "extended_entities": {"media": [{
            "type": "animated_gif",
            "media_url_https": "https://img/gif.jpg",
            "video_info": {"variants": [
                {"content_type": "video/mp4", "bitrate": 100, "url": "https://v/g.mp4"}
            ]}
        }]}
    }));
    let tweet = parse_tweet_result(&result, 0).unwrap();
    let m = &tweet.media[0];
    assert_eq!(m.media_type, "animated_gif");
    assert_eq!(m.url, "https://v/g.mp4");
    assert_eq!(m.poster_url.as_deref(), Some("https://img/gif.jpg"));
    assert_eq!(m.variants.as_ref().unwrap().len(), 1);
}

#[test]
fn photo_media_keeps_alt_text_but_no_poster_or_variants() {
    let result = tweet_result(serde_json::json!({
        "full_text": "pic",
        "created_at": "t",
        "lang": "en",
        "extended_entities": {"media": [{
            "type": "photo",
            "media_url_https": "https://img/p.jpg",
            "ext_alt_text": "a red square",
            "original_info": {"width": 100, "height": 100}
        }]}
    }));
    let tweet = parse_tweet_result(&result, 0).unwrap();
    let m = &tweet.media[0];
    assert_eq!(m.media_type, "photo");
    assert_eq!(m.url, "https://img/p.jpg");
    assert_eq!(m.alt_text.as_deref(), Some("a red square"));
    assert!(m.poster_url.is_none());
    assert!(m.variants.is_none());
    assert!(m.duration_millis.is_none());
}

#[test]
fn tweet_metadata_flags_mentions_reply_edit_control() {
    let mut result = tweet_result(serde_json::json!({
        "full_text": "hi @bob",
        "created_at": "t",
        "lang": "en",
        "possibly_sensitive": false,
        "display_text_range": [0, 8],
        "favorited": true,
        "retweeted": false,
        "bookmarked": true,
        "in_reply_to_status_id_str": "55",
        "in_reply_to_user_id_str": "777",
        "in_reply_to_screen_name": "bob",
        "entities": {"user_mentions": [{
            "id_str": "777",
            "screen_name": "bob",
            "name": "Bob",
            "indices": [3, 7]
        }]}
    }));
    result["edit_control"] =
        serde_json::json!({"edit_tweet_ids": ["t1"], "editable_until_msecs": "1"});
    result["has_birdwatch_notes"] = serde_json::json!(true);
    let tweet = parse_tweet_result(&result, 0).unwrap();
    // Explicit false preserved (retweeted), absent-vs-false distinction kept.
    assert_eq!(tweet.possibly_sensitive, Some(false));
    assert_eq!(tweet.display_text_range, Some(vec![0, 8]));
    assert_eq!(tweet.favorited, Some(true));
    assert_eq!(tweet.retweeted, Some(false));
    assert_eq!(tweet.bookmarked, Some(true));
    assert_eq!(tweet.in_reply_to_user_id.as_deref(), Some("777"));
    assert_eq!(tweet.in_reply_to_screen_name.as_deref(), Some("bob"));
    assert_eq!(tweet.edit_control.unwrap()["editable_until_msecs"], "1");
    assert_eq!(tweet.has_birdwatch_notes, Some(true));
    let mentions = tweet.mentions.unwrap();
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].id, "777");
    assert_eq!(mentions[0].screen_name.as_deref(), Some("bob"));
    assert_eq!((mentions[0].start, mentions[0].end), (Some(3), Some(7)));
}

#[test]
fn absent_metadata_stays_absent() {
    let result = tweet_result(serde_json::json!({
        "full_text": "plain",
        "created_at": "t",
        "lang": "en",
    }));
    let tweet = parse_tweet_result(&result, 0).unwrap();
    let v = serde_json::to_value(&tweet).unwrap();
    for key in [
        "possibly_sensitive",
        "display_text_range",
        "mentions",
        "in_reply_to_user_id",
        "in_reply_to_screen_name",
        "edit_control",
        "favorited",
        "retweeted",
        "bookmarked",
        "has_birdwatch_notes",
    ] {
        assert!(v.get(key).is_none(), "{key} must be absent");
    }
    assert!(tweet.author.following.is_none());
    assert!(tweet.author.followed_by.is_none());
}

#[test]
fn quoted_and_retweeted_tweets_carry_their_own_metadata() {
    // Media/metadata on the QUOTED tweet must not leak onto the parent.
    let quoted = tweet_result(serde_json::json!({
        "full_text": "inner",
        "created_at": "t",
        "lang": "en",
        "favorited": true,
        "extended_entities": {"media": [{
            "type": "photo", "media_url_https": "https://img/q.jpg",
            "ext_alt_text": "inner alt"
        }]}
    }));
    let mut outer_legacy = serde_json::json!({
        "full_text": "outer",
        "created_at": "t",
        "lang": "en",
    });
    let mut result = tweet_result(std::mem::take(&mut outer_legacy));
    result["quoted_status_result"] = serde_json::json!({"result": quoted});
    let tweet = parse_tweet_result(&result, 0).unwrap();
    assert!(tweet.favorited.is_none());
    assert!(tweet.media.is_empty());
    let q = tweet.quoted_tweet.unwrap();
    assert_eq!(q.favorited, Some(true));
    assert_eq!(q.media[0].alt_text.as_deref(), Some("inner alt"));
}

#[test]
fn user_profile_exposes_following_and_followed_by() {
    let user = serde_json::json!({
        "rest_id": "u9",
        "core": {"name": "Grace", "screen_name": "grace"},
        "legacy": {
            "name": "Grace",
            "screen_name": "grace",
            "following": true,
            "followed_by": false,
        },
    });
    let p = parse_user_result(&user).unwrap();
    assert_eq!(p.following, Some(true));
    // Explicit false is data, not absence.
    assert_eq!(p.followed_by, Some(false));

    let no_flags = serde_json::json!({
        "rest_id": "u8",
        "core": {"name": "No", "screen_name": "no"},
        "legacy": {"name": "No", "screen_name": "no"},
    });
    let p2 = parse_user_result(&no_flags).unwrap();
    assert!(p2.following.is_none());
    assert!(p2.followed_by.is_none());
    let v = serde_json::to_value(&p2).unwrap();
    assert!(v.get("following").is_none());
    assert!(v.get("followed_by").is_none());
}

#[test]
fn tweet_author_carries_viewer_follow_state() {
    let mut result = tweet_result(serde_json::json!({
        "full_text": "hi",
        "created_at": "t",
        "lang": "en",
    }));
    result["core"]["user_results"]["result"]["legacy"] = serde_json::json!({
        "following": true,
        "followed_by": true,
    });
    let tweet = parse_tweet_result(&result, 0).unwrap();
    assert_eq!(tweet.author.following, Some(true));
    assert_eq!(tweet.author.followed_by, Some(true));
}
