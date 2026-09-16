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
    // P6.1 user-timeline variants — starting points from agentic-x's
    // live-probed 2026-07-26 sweep (variables shape, envelope root, and a
    // non-empty response all verified there), NOT yet re-verified against
    // this repo's own `doctor --refresh`. All three answer under the SAME
    // envelope root as UserTweets
    // (`data.user.result.timeline.timeline.instructions`, with the
    // `timeline_v2` fallback) and NONE is behind the tx-id wall — that is
    // precisely why UserRepliesTimeline is preferred over the gated
    // UserTweetsAndReplies (which interleaves replies with posts AND needs
    // a fresh x-client-transaction-id per request, see twr-tx::GATED_OPS):
    // replies-only tab, cheaper (no tx mint) and resilient when the
    // tx generator rots. Variables per agentic-x `user_tab_variables`
    // (LIVE-CAPTURED 2026-07-26): `{userId, count, includePromotedContent:
    // false, withClientEventToken: false, withBirdwatchNotes: false,
    // withVoice: true}` (+ cursor when paging). X validates variables
    // strictly — do NOT copy UserTweets' `withQuickPromoteEligibility...`
    // bundle onto these ops (known 404 cause).
    ("UserRepliesTimeline", "pb6crFNr_CyRiKv4vRZWYQ"),
    ("UserMedia", "6k_h0NmaKHYxL0lScGLJSw"),
    // P6.2 pin — operation name CONFIRMED in the research corpus
    // (twitter-internal-api-doc deck GraphQL.json + API.json agree:
    // PinTweet/UnpinTweet mutations). Query IDs from the deck capture:
    // PinTweet `VIHsNu89pK-kW35JpHq7Xw`, UnpinTweet `BhKei844ypCyLYCg0nwigw`.
    // UNVERIFIED against this repo's own `doctor --refresh` — starting
    // points, re-resolve live before trusting (they rot every 2–4 weeks).
    // NOTE: mute/block have NO GraphQL op in any reference (no MuteUser/
    // BlockUser anywhere in xeepy/Rettiwt-API/agentic-x/deck-GraphQL or
    // xfetch/bird/peep). They ride the 1.1 REST endpoints instead
    // (`mutes/users/create|destroy`, `blocks/create|destroy` — deck
    // v1.1.json, form-POST like the existing follow/unfollow path), so no
    // FALLBACK_QUERY_IDS entry exists or is needed for them. If a future
    // live capture surfaces GraphQL mute/block ops, add them here then.
    ("PinTweet", "VIHsNu89pK-kW35JpHq7Xw"),
    ("UnpinTweet", "BhKei844ypCyLYCg0nwigw"),
    // P6.4 long-form — VERBATIM from Rettiwt-API `ListRequests`-sibling
    // `TweetRequests.postNote` (pinned blob 4f11105, file
    // src/requests/Tweet.ts; byte-identical at
    // cdn.jsdelivr.net/npm/rettiwt-api@7.1.3/src/requests/Tweet.ts;
    // independently fetched + verified by c1). QueryId
    // `_eeuQKX1-VyRP_ROM-GN7g` matches this bead's cited ID. Variables
    // `{tweet_text, media?, semantic_annotation_ids: [],
    // disallowed_reply_options: null}` (NO dark_request, NO reply/quote
    // path in postNote — reply/quote long-form UNCONFIRMED). Features:
    // 33-flag Rettiwt-verbatim set incl. longform_notetweets_creation_enabled
    // (absent from repo DEFAULT_FEATURES) — consult `note_features()`,
    // NOT compact_features defaults. Body: NO queryId-in-body, NO
    // fieldToggles (do NOT copy the list-mutation pattern here).
    ("CreateNoteTweet", "_eeuQKX1-VyRP_ROM-GN7g"),
    // P6.3 list management — operation names + query IDs CONFIRMED in the
    // research corpus (twitter-internal-api-doc deck GraphQL.json and both
    // API.json captures agree on all seven; deck GraphQL.md ChangeLog lists
    // the same op names). UNVERIFIED against this repo's own
    // `doctor --refresh` — starting points, re-resolve live (rot 2–4 wks).
    // Disambiguation notes for the two ambiguous cases:
    // - "follow a list" is ListSubscribe/ListUnsubscribe (NOT FollowList/
    //   UnfollowList — no such op exists in ANY reference; the plan §13.3
    //   table used FollowList as shorthand). Subscribe = follow someone
    //   else's list; engagement-tier like user-follow.
    // - "pin a list" has NO PinList/UnpinList op (deck-wide grep: only
    //   Tweet/Reply/Timeline/Conversation pin ops exist). The list-sidebar
    //   equivalent is UpdatePinnedTimelines (`AtN-0mKI3fXXmxzYYk1Wqw`) —
    //   variables shape unconfirmed in the corpus, so list-pin ships behind
    //   that op with a best-effort `{listId, pinned}` shape flagged for
    //   live confirmation; c1 must verify/replace before trusting.
    ("CreateList", "AkWrYT3WjoBVkzbnbvLkhg"),
    ("UpdateList", "6fJbXehrO7k4iSr_TK1U2Q"),
    ("DeleteList", "UnN9Th1BDbeLjpgjGSpL3Q"),
    ("ListAddMember", "F4BvT6Af48GSxTgqNLIdrQ"),
    ("ListRemoveMember", "llA1p2EP5J3gReAQQsW1vw"),
    ("ListSubscribe", "1B3FqCK_7uU6W_AHqWBx8A"),
    ("ListUnsubscribe", "8gopUa1KU_9afKsxI9Y_Rg"),
    ("UpdatePinnedTimelines", "AtN-0mKI3fXXmxzYYk1Wqw"),
];

/// Shipped EXTRA-rotation fallbacks (layer 3): the older/alternate query ID
/// per op, tried when the baseline 404s. Sourced from the reference corpus
/// (twitter-internal-api-doc captures predate bird/x-cli-go's live-scraped
/// values); re-verified or replaced by `doctor --refresh` on drift.
pub const EXTRA_FALLBACK_IDS: &[(&str, &str)] = &[
    ("ListMembers", "ljlktihgwXeYTfHwwiPj5A"),
    ("ListMembers", "Bnhcen0kdsMAU1tW7U79qQ"),
    ("ListByRestId", "EAARFZGlY-JHdLJbKZAA5g"),
    ("ListByRestId", "Tzkkg-NaBi_y1aAUUb6_eQ"),
    ("ListAddMember", "EadD8ivrhZhYQr2pDmCpjA"),
    ("CreateList", "4lSOF4GqldI-NbiFET4ofQ"),
    ("UpdateList", "UzVGAR_brbQw1n3mH_PqRA"),
];

/// Seed an [`crate::ExtraRotation`] map with the shipped alternates.
/// Multiple entries per op accumulate (rotation order = listed order).
pub fn seeded_extra_rotation() -> crate::ExtraRotation {
    let mut extra = crate::ExtraRotation::new();
    for (op, qid) in EXTRA_FALLBACK_IDS {
        extra
            .entry((*op).to_string())
            .or_default()
            .push((*qid).to_string());
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

/// Per-op feature OVERRIDES: narrower schema than the repo defaults, taken
/// VERBATIM from Rettiwt-API `ListRequests` (a live-shaped working caller:
/// e.g. its `create()` sends profile_label=true, redirect=FALSE,
/// tipjar=false, verified=false, skip_user_profile_image=false,
/// timeline_nav=true).
///
/// NOT a proven 214 fix — schema hygiene only. Evidence against the
/// strict-features theory: PinTweet sends the full 16-flag defaults and
/// succeeds live on the same session that 214s CreateList. The deck
/// `GraphQL.json` even disagrees with Rettiwt on one flag here
/// (deck says redirect=true, Rettiwt sends false); Rettiwt wins because it
/// is a working caller and the deck is a static capture. The 214 root
/// cause is still UNRESOLVED — DevTools capture of x.com/i/lists/create
/// (URL + Request Payload variables + features) is ground truth.
/// `compact_features` consults this FIRST and skips defaults/extras for
/// listed ops. DeleteList sends `{}` (deck declares zero features;
/// Rettiwt's delete sends no features key at all).
pub fn feature_overrides(operation: &str) -> Option<&'static [(&'static str, bool)]> {
    match operation {
        "CreateList"
        | "UpdateList"
        | "ListAddMember"
        | "ListRemoveMember"
        | "ListSubscribe"
        | "ListUnsubscribe"
        | "UpdatePinnedTimelines" => Some(&[
            ("profile_label_improvements_pcf_label_in_post_enabled", true),
            ("responsive_web_profile_redirect_enabled", false),
            ("rweb_tipjar_consumption_enabled", false),
            ("verified_phone_label_enabled", false),
            (
                "responsive_web_graphql_skip_user_profile_image_extensions_enabled",
                false,
            ),
            ("responsive_web_graphql_timeline_navigation_enabled", true),
        ]),
        "DeleteList" => Some(&[]),
        // CreateNoteTweet: Rettiwt postNote verbatim (33 flags; the only
        // mutation in the repo whose feature set comes from a verified
        // working caller rather than deck inference or repo defaults).
        // Includes longform_notetweets_creation_enabled (absent from
        // DEFAULT_FEATURES) and content_disclosure_*/post_ctas_*/jetfuel/
        // grok/annotations flags the plain CreateTweet path never sends.
        "CreateNoteTweet" => Some(&[
            ("premium_content_api_read_enabled", false),
            ("communities_web_enable_tweet_community_results_fetch", true),
            ("c9s_tweet_anatomy_moderator_badge_enabled", true),
            (
                "responsive_web_grok_analyze_button_fetch_trends_enabled",
                false,
            ),
            ("responsive_web_grok_analyze_post_followups_enabled", true),
            ("responsive_web_jetfuel_frame", true),
            ("responsive_web_grok_share_attachment_enabled", true),
            ("responsive_web_grok_annotations_enabled", true),
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
            ("content_disclosure_indicator_enabled", true),
            ("content_disclosure_ai_generated_indicator_enabled", true),
            ("responsive_web_grok_show_grok_translated_post", true),
            ("responsive_web_grok_analysis_button_from_backend", true),
            ("post_ctas_fetch_enabled", true),
            ("longform_notetweets_rich_text_read_enabled", true),
            ("longform_notetweets_inline_media_enabled", false),
            ("profile_label_improvements_pcf_label_in_post_enabled", true),
            ("responsive_web_profile_redirect_enabled", false),
            ("rweb_tipjar_consumption_enabled", false),
            ("verified_phone_label_enabled", false),
            ("articles_preview_enabled", true),
            (
                "responsive_web_grok_community_note_auto_translation_is_enabled",
                false,
            ),
            (
                "responsive_web_graphql_skip_user_profile_image_extensions_enabled",
                false,
            ),
            ("freedom_of_speech_not_reach_fetch_enabled", true),
            ("standardized_nudges_misinfo", true),
            (
                "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled",
                true,
            ),
            ("responsive_web_grok_image_annotation_enabled", true),
            ("responsive_web_grok_imagine_annotation_enabled", true),
            ("responsive_web_graphql_timeline_navigation_enabled", true),
            ("responsive_web_enhance_cards_enabled", false),
            ("longform_notetweets_creation_enabled", true),
        ]),
        _ => None,
    }
}

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
/// Ops with a [`feature_overrides`] entry use ONLY their deck-declared
/// schema (strict-validation 214 guard) — defaults/extras are skipped.
pub fn compact_features(operation: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    if let Some(schema) = feature_overrides(operation) {
        for (k, v) in schema {
            if *v {
                out.insert((*k).to_string(), serde_json::Value::Bool(true));
            }
        }
        return out;
    }
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
    fn baseline_has_22_plus_4_list_plus_2_user_tab_plus_2_pin_plus_8_list_mgmt_plus_1_note() {
        assert_eq!(FALLBACK_QUERY_IDS.len(), 39);
        for op in [
            "ListOwnerships",
            "ListMemberships",
            "ListMembers",
            "ListByRestId",
            "UserRepliesTimeline",
            "UserMedia",
            "PinTweet",
            "UnpinTweet",
            "CreateList",
            "UpdateList",
            "DeleteList",
            "ListAddMember",
            "ListRemoveMember",
            "ListSubscribe",
            "ListUnsubscribe",
            "UpdatePinnedTimelines",
            "CreateNoteTweet",
        ] {
            assert!(fallback_query_id(op).is_some(), "{op}");
        }
    }

    #[test]
    fn note_tweet_uses_rettiwt_verbatim_schema() {
        // 25 true-valued flags (out of 35 total incl. false-valued) —
        // byte-compared 1-1 against Rettiwt postNote by the reviewer
        // (cdn.jsdelivr.net/npm/rettiwt-api@7.1.3/src/requests/Tweet.ts):
        // identical true-set to postNote PLUS longform_notetweets_
        // creation_enabled (reviewer-added, postNote predates it; harmless —
        // PinTweet with full defaults proves features aren't strictly
        // validated) and MINUS nothing. Must NOT contain tweetypie_unmention
        // (a CreateTweet-only flag never in postNote).
        let f = compact_features("CreateNoteTweet");
        assert_eq!(f.len(), 26);
        assert!(f.contains_key("longform_notetweets_creation_enabled"));
        assert!(f.contains_key("c9s_tweet_anatomy_moderator_badge_enabled"));
        assert!(f.contains_key("content_disclosure_indicator_enabled"));
        assert!(f.contains_key("post_ctas_fetch_enabled"));
        assert!(!f.contains_key("tweetypie_unmention_optimization_enabled"));
    }

    #[test]
    fn extra_fallbacks_cover_rotating_list_ops() {
        let extra = seeded_extra_rotation();
        // Rettiwt-sourced alternates accumulate after the doc-capture ones.
        assert_eq!(
            extra.get("ListMembers").map(|v| v.as_slice()),
            Some(
                [
                    "ljlktihgwXeYTfHwwiPj5A".to_string(),
                    "Bnhcen0kdsMAU1tW7U79qQ".to_string()
                ]
                .as_slice()
            )
        );
        assert_eq!(
            extra.get("ListByRestId").map(|v| v.as_slice()),
            Some(
                [
                    "EAARFZGlY-JHdLJbKZAA5g".to_string(),
                    "Tzkkg-NaBi_y1aAUUb6_eQ".to_string()
                ]
                .as_slice()
            )
        );
        assert_eq!(
            extra.get("ListAddMember").map(|v| v.as_slice()),
            Some(["EadD8ivrhZhYQr2pDmCpjA".to_string()].as_slice())
        );
        assert_eq!(
            extra.get("CreateList").map(|v| v.as_slice()),
            Some(["4lSOF4GqldI-NbiFET4ofQ".to_string()].as_slice())
        );
        assert_eq!(
            extra.get("UpdateList").map(|v| v.as_slice()),
            Some(["UzVGAR_brbQw1n3mH_PqRA".to_string()].as_slice())
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

    #[test]
    fn list_mutations_use_rettiwt_schema_not_defaults() {
        // Schema hygiene (NOT a proven 214 fix): Rettiwt-verbatim
        // 2-true-flag schema, none of the 16 repo defaults (e.g.
        // view_counts_everywhere must be ABSENT).
        for op in [
            "CreateList",
            "UpdateList",
            "ListAddMember",
            "ListRemoveMember",
            "ListSubscribe",
            "ListUnsubscribe",
            "UpdatePinnedTimelines",
        ] {
            let f = compact_features(op);
            assert_eq!(f.len(), 2, "{op}: {f:?}");
            assert!(f.contains_key("profile_label_improvements_pcf_label_in_post_enabled"));
            assert!(f.contains_key("responsive_web_graphql_timeline_navigation_enabled"));
            assert!(
                !f.contains_key("responsive_web_profile_redirect_enabled"),
                "{op}"
            );
            assert!(
                !f.contains_key("view_counts_everywhere_api_enabled"),
                "{op}"
            );
        }
        // DeleteList declares zero features.
        assert!(compact_features("DeleteList").is_empty());
        // Untouched ops keep the defaults.
        assert!(
            compact_features("SearchTimeline").contains_key("view_counts_everywhere_api_enabled")
        );
    }
}
