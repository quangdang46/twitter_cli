//! Domain structs, ported field-for-field from `tmp/_research/twitter-py/twitter_cli/models.py`
//! (see COMPREHENSIVEPLANFORTWITTERCLI.md §7 — field names are kept identical
//! to the Python originals on purpose, so fixture-parity tests in a later
//! bead can deep-compare JSON output without a translation layer).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Author {
    pub id: String,
    pub name: String,
    pub screen_name: String,
    #[serde(default)]
    pub profile_image_url: String,
    #[serde(default)]
    pub verified: bool,
    /// Viewer relationship from `legacy.following` / `legacy.followed_by`
    /// (issue #14). `None` when X does not supply the key — a missing field
    /// is not evidence of "not following", so no `false` default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub following: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed_by: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    #[serde(default)]
    pub likes: i64,
    #[serde(default)]
    pub retweets: i64,
    #[serde(default)]
    pub replies: i64,
    #[serde(default)]
    pub quotes: i64,
    #[serde(default)]
    pub views: i64,
    #[serde(default)]
    pub bookmarks: i64,
}

/// One playback variant from `video_info.variants` (issue #13) — mp4
/// renditions AND the HLS playlist, kept in supplied order with url,
/// content type, and bitrate when present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaVariant {
    #[serde(default)]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TweetMedia {
    /// `"photo" | "video" | "animated_gif"`
    #[serde(rename = "type")]
    pub media_type: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    /// `id_str` — kept as a string, never parsed to int (issue #13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_key: Option<String>,
    /// `media_url_https` — poster frame for video/animated_gif only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_url: Option<String>,
    /// `ext_alt_text` accessibility description when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    /// `video_info.duration_millis` when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_millis: Option<i64>,
    /// All `video_info.variants` verbatim (mp4 + HLS), supplied order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<MediaVariant>>,
}

/// One `legacy.entities.user_mentions[]` entry (issue #13): structured
/// mention with id, handle, display name, and text offsets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mention {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<i64>,
}

/// `quoted_tweet` is `Box`ed because `Tweet` is self-referential (a quote
/// tweet embeds another `Tweet`) — Python's dataclass didn't need this since
/// it isn't statically sized, but Rust does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tweet {
    pub id: String,
    pub text: String,
    pub author: Author,
    pub metrics: Metrics,
    pub created_at: String,
    #[serde(default)]
    pub media: Vec<TweetMedia>,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub is_retweet: bool,
    #[serde(default)]
    pub lang: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retweeted_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_tweet: Option<Box<Tweet>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub article_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub article_text: Option<String>,
    #[serde(default)]
    pub is_subscriber_only: bool,
    #[serde(default)]
    pub is_promoted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to_status_id: Option<String>,
    /// Raw `grok_translated_post_with_availability` object when X supplies
    /// it (issue #7) — kept verbatim, never flattened; `text`/`lang` stay
    /// the original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grok_translated_post_with_availability: Option<serde_json::Value>,
    /// Raw `grok_translated_post` object (`{entities, translation}`) — the
    /// current wire name for the same Grok translation payload (issue #24,
    /// live-verified 2026: X returns this shape on TweetDetail/HomeTimeline
    /// where older captures had the `_with_availability` wrapper). Kept
    /// verbatim under its own key; absent stays absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grok_translated_post: Option<serde_json::Value>,
    /// `legacy.possibly_sensitive` — explicit `false` is preserved; absent
    /// stays absent (issue #13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub possibly_sensitive: Option<bool>,
    /// `legacy.display_text_range` `[start, end]` char offsets when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_text_range: Option<Vec<i64>>,
    /// `legacy.entities.user_mentions` structured mentions (issue #13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<Mention>>,
    /// `legacy.in_reply_to_user_id_str` — recipient of a reply (issue #13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to_user_id: Option<String>,
    /// `legacy.in_reply_to_screen_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to_screen_name: Option<String>,
    /// Raw `edit_control` object, preserved verbatim when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_control: Option<serde_json::Value>,
    /// `legacy.favorited` — viewer liked this tweet (issue #13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorited: Option<bool>,
    /// `legacy.retweeted` — viewer retweeted this tweet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retweeted: Option<bool>,
    /// `legacy.bookmarked` — viewer bookmarked this tweet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmarked: Option<bool>,
    /// Top-level `has_birdwatch_notes` indicator — presence flag only,
    /// never the note contents (issue #13).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_birdwatch_notes: Option<bool>,
}

/// A Twitter List (owned or followed). NOT a Tweet — its own type, mirroring
/// bird/xfetch's `TwitterList` (`id_str/name/description/member_count/
/// subscriber_count/mode/user_results`). `owner` is a lightweight handle
/// triple (full profile needs a separate `user` call).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListOwner {
    pub id: String,
    #[serde(default)]
    pub screen_name: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwitterList {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub member_count: i64,
    #[serde(default)]
    pub subscriber_count: i64,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<ListOwner>,
}

/// One notification event from the URT notifications REST endpoint
/// (`/i/api/2/notifications/{all,mentions}.json`). NOT a Tweet at the top
/// level: an event (`like`/`retweet`/`follow`/`mention`/`reply`/`quote`…)
/// with actor(s) and, when the event references one, the target tweet.
/// Global tweet/user objects are resolved to ids at parse time (full bodies
/// stay in `tweets`, same as xfetch's `Notification{fromUsers,targetTweets}`
/// split) so the envelope stays small and agent-scannable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationActor {
    pub id: String,
    #[serde(default)]
    pub screen_name: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationEvent {
    /// Server `icon` name lowercased (`heart`/`retweet`/`person`/`mention`…),
    /// or the timeline-entry fallback id when no icon is present.
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<NotificationActor>,
    /// Referenced tweet ids (globalObjects-resolved; bodies in `tweets`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tweet_ids: Vec<String>,
    #[serde(default)]
    pub timestamp_ms: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A DM conversation (own family — NOT a Tweet, NOT a NotificationEvent:
/// inbound social signal about your content vs. private correspondence are
/// fundamentally different concepts; see bead o1l.5.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmConversation {
    pub id: String,
    #[serde(default)]
    pub participants: Vec<NotificationActor>,
    #[serde(default)]
    pub last_message: Option<String>,
    #[serde(default)]
    pub last_timestamp_ms: String,
    #[serde(default)]
    pub unread_count: i64,
}

/// One DM event within a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DmMessage {
    pub id: String,
    #[serde(default)]
    pub sender_id: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub timestamp_ms: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookmarkFolder {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub name: String,
    pub screen_name: String,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub followers_count: i64,
    #[serde(default)]
    pub following_count: i64,
    #[serde(default)]
    pub tweets_count: i64,
    #[serde(default)]
    pub likes_count: i64,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub profile_image_url: String,
    #[serde(default)]
    pub created_at: String,
    /// Viewer relationship from `legacy.following` / `legacy.followed_by`
    /// (issue #14) — omitted when X does not supply the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub following: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed_by: Option<bool>,
}
