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
}
