//! Shipped baseline: the 22 `FALLBACK_QUERY_IDS` from plan §1.2, ported
//! verbatim from the Python original's `graphql.py`, plus the 21 default
//! FEATURES flags and per-call extras.
//!
//! These are the layer-1 fallback of the 4-layer resolver (see
//! [`crate::resolve`]). IDs rot every 2–4 weeks — never trust a committed ID,
//! re-scrape in CI (plan §2.1).

/// `(operation_name, query_id)` pairs, verbatim from `graphql.py`.
pub const FALLBACK_QUERY_IDS: &[(&str, &str)] = &[
    ("HomeTimeline", "c-CzHF1LboFilMpsx4ZCrQ"),
    ("HomeLatestTimeline", "BKB7oi212Fi7kQtCBGE4zA"),
    ("UserByScreenName", "1VOOyvKkiI3FMmkeDNxM9A"),
    ("UserTweets", "q6xj5bs0hapm9309hexA_g"),
    ("TweetDetail", "xd_EMdYvB9hfZsZ6Idri0w"),
    ("Likes", "lIDpu_NWL7_VhimGGt0o6A"),
    ("SearchTimeline", "VhUd6vHVmLBcw0uX-6jMLA"),
    ("Bookmarks", "2neUNDqrrFzbLui8yallcQ"),
    ("ListLatestTweetsTimeline", "RlZzktZY_9wJynoepm8ZsA"),
    ("Followers", "IOh4aS6UdGWGJUYTqliQ7Q"),
    ("Following", "zx6e-TLzRkeDO_a7p4b3JQ"),
    ("CreateTweet", "IID9x6WsdMnTlXnzXGq8ng"),
    ("DeleteTweet", "VaenaVgh5q5ih7kvyVjgtg"),
    ("FavoriteTweet", "lI07N6Otwv1PhnEgXILM7A"),
    ("UnfavoriteTweet", "ZYKSe-w7KEslx3JhSIk5LA"),
    ("CreateRetweet", "ojPdsZsimiJrUGLR1sjUtA"),
    ("DeleteRetweet", "iQtK4dl5hBmXewYZuEOKVw"),
    ("CreateBookmark", "aoDbu3RHznuiSkQ9aNM67Q"),
    ("DeleteBookmark", "Wlmlj2-xzyS1GN3a6cj-mQ"),
    ("TweetResultByRestId", "7xflPyRiUxGVbJd4uWmbfg"),
    ("BookmarkFoldersSlice", "i78YDd0Tza-dV4SYs58kRg"),
    ("BookmarkFolderTimeline", "hNY7X2xE2N7HVF6Qb_mu6w"),
    // P6.1 list reads — starting points mined from public reference
    // implementations, NOT yet verified against this repo's own
    // `doctor --refresh` (they rot every 2–4 weeks, plan §12 risk 2).
    // Owned-vs-followed disambiguation: two SEPARATE ops, confirmed by
    // bird (`getOwnedLists` vs `getListMemberships`) and xfetch
    // (`getUserLists` → ListOwnerships only): `ListOwnerships` returns
    // lists the `userId` OWNS, `ListMemberships` returns lists they are a
    // MEMBER of (followed/subscribed are a third thing — `ListByRestId`
    // resolves one list at a time, no bulk "followed" op exists in any
    // reference). Both take `{userId, count, isListMembershipShown}` and
    // walk `data.user.result.timeline.timeline.instructions` for
    // `content.itemContent.list` entries. Every P6 bead must re-resolve
    // these via the 4-layer resolver before landing (TWR_QID_* pin >
    // disk cache > EXTRA rotation > live rescrape).
    ("ListOwnerships", "wQcOSjSQ8NtgxIwvYl1lMg"),
    ("ListMemberships", "BlEXXdARdSeL_0KyKHHvvg"),
    // ListMembers (members of one list): x-cli-go's baseline
    // `H_0zFfjp73xGZrJpY-C2IQ`; twitter-internal-api-doc's older capture
    // `ljlktihgwXeYTfHwwiPj5A` is kept as the EXTRA-rotation fallback.
    // Payload: `data.list.members_timeline.timeline.instructions` with
    // `content.itemContent.user_results.result` entries (members ARE
    // users — reuse parse_user_result, no new model type).
    ("ListMembers", "H_0zFfjp73xGZrJpY-C2IQ"),
    // ListByRestId (one list by ID, for P6.3's create/edit verification):
    // bird's baseline `wXzyA5vM_aVkBL9G8Vp3kw`; the doc's older
    // `EAARFZGlY-JHdLJbKZAA5g` becomes the EXTRA fallback. Payload is a
    // bare `data.list` object (not a timeline), parsed by parse_list_result.
    ("ListByRestId", "wXzyA5vM_aVkBL9G8Vp3kw"),
];

/// Shipped EXTRA-rotation fallbacks (layer 3): the older/alternate query ID
/// per op, tried when the baseline 404s. Sourced from the reference corpus
/// (twitter-internal-api-doc captures predate bird/x-cli-go's live-scraped
/// values); re-verified or replaced by `doctor --refresh` on drift.
pub const EXTRA_FALLBACK_IDS: &[(&str, &str)] = &[
    ("ListMembers", "ljlktihgwXeYTfHwwiPj5A"),
    ("ListByRestId", "EAARFZGlY-JHdLJbKZAA5g"),
];

/// Seed an [`crate::ExtraRotation`] map with the shipped alternates.
pub fn seeded_extra_rotation() -> crate::ExtraRotation {
    let mut extra = crate::ExtraRotation::new();
    for (op, qid) in EXTRA_FALLBACK_IDS {
        extra.insert((*op).to_string(), vec![(*qid).to_string()]);
    }
    extra
}

pub fn fallback_query_id(operation: &str) -> Option<&'static str> {
    FALLBACK_QUERY_IDS
        .iter()
        .find(|(op, _)| *op == operation)
        .map(|(_, qid)| *qid)
}

/// The 20 default feature flags from `graphql.py::_DEFAULT_FEATURES` (the plan
/// says 21, but the Python source has exactly 20 — counted, not guessed).
/// Serialized with `true` values only at request time (false values are
/// stripped to avoid HTTP 414 — see [`compact_features`]).
pub const DEFAULT_FEATURES: &[(&str, bool)] = &[
    ("responsive_web_graphql_exclude_directive_enabled", true),
    ("verified_phone_label_enabled", false),
    ("creator_subscriptions_tweet_preview_api_enabled", true),
    ("responsive_web_graphql_timeline_navigation_enabled", true),
    (
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled",
        false,
    ),
    ("c9s_tweet_anatomy_moderator_badge_enabled", true),
    ("tweetypie_unmention_optimization_enabled", true),
    ("responsive_web_edit_tweet_api_enabled", true),
    (
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled",
        true,
    ),
    ("view_counts_everywhere_api_enabled", true),
    ("longform_notetweets_consumption_enabled", true),
    (
        "responsive_web_twitter_article_tweet_consumption_enabled",
        true,
    ),
    ("tweet_awards_web_tipping_enabled", false),
    ("longform_notetweets_rich_text_read_enabled", true),
    ("longform_notetweets_inline_media_enabled", true),
    ("rweb_video_timestamps_enabled", true),
    ("responsive_web_media_download_video_enabled", true),
    ("freedom_of_speech_not_reach_fetch_enabled", true),
    ("standardized_nudges_misinfo", true),
    ("responsive_web_enhance_cards_enabled", false),
];

/// Per-call extras from plan §1.2 (appended to the defaults per operation).
pub fn extra_features(operation: &str) -> &'static [(&'static str, bool)] {
    match operation {
        // fetch_user extras: hidden/tipjar/subscriptions/highlights/
        // article-notes/gift toggles.
        "UserByScreenName" => &[
            ("hidden_profile_subscriptions_enabled", true),
            ("tipjar_enabled", true),
            ("subscriptions_verification_info_enabled", true),
            ("highlights_tweets_tab_ui_enabled", true),
            ("article_notes_tab_enabled", true),
            ("gift_button_enabled", true),
        ],
        // fetch_article extras.
        "Article" => &[
            ("articles_preview_enabled", true),
            ("responsive_web_article_rich_content_enabled", true),
        ],
        // tweet_detail extras.
        "TweetDetail" => &[("withArticleRichContentState", true)],
        _ => &[],
    }
}

/// Merge defaults + per-op extras, dropping false values (414 guard).
pub fn compact_features(operation: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for (k, v) in DEFAULT_FEATURES.iter().chain(extra_features(operation)) {
        if *v {
            out.insert((*k).to_string(), serde_json::Value::Bool(true));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_has_22_plus_4_list_ops() {
        assert_eq!(FALLBACK_QUERY_IDS.len(), 26);
        for op in [
            "ListOwnerships",
            "ListMemberships",
            "ListMembers",
            "ListByRestId",
        ] {
            assert!(fallback_query_id(op).is_some(), "{op}");
        }
    }

    #[test]
    fn extra_fallbacks_cover_rotating_list_ops() {
        let extra = seeded_extra_rotation();
        assert_eq!(
            extra.get("ListMembers").map(|v| v.as_slice()),
            Some(["ljlktihgwXeYTfHwwiPj5A".to_string()].as_slice())
        );
        assert_eq!(
            extra.get("ListByRestId").map(|v| v.as_slice()),
            Some(["EAARFZGlY-JHdLJbKZAA5g".to_string()].as_slice())
        );
    }

    #[test]
    fn defaults_have_20_flags() {
        assert_eq!(DEFAULT_FEATURES.len(), 20);
    }

    #[test]
    fn compact_features_strips_false_to_avoid_414() {
        let features = compact_features("SearchTimeline");
        // All 4 false-valued defaults must be gone.
        for dead in [
            "verified_phone_label_enabled",
            "responsive_web_graphql_skip_user_profile_image_extensions_enabled",
            "tweet_awards_web_tipping_enabled",
            "responsive_web_enhance_cards_enabled",
        ] {
            assert!(!features.contains_key(dead), "{dead}");
        }
        assert!(features.contains_key("view_counts_everywhere_api_enabled"));
    }

    #[test]
    fn extras_apply_per_op() {
        assert!(!extra_features("SearchTimeline").iter().any(|_| true));
        let user = compact_features("UserByScreenName");
        assert!(user.contains_key("hidden_profile_subscriptions_enabled"));
        let detail = compact_features("TweetDetail");
        assert!(detail.contains_key("withArticleRichContentState"));
    }
}
