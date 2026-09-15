//! Human table renderer (bead twitter_cli-5o3.5.1).
//!
//! `#/Author/Tweet/Stats/Score/Time` columns, 120-char text truncation
//! unless `--full-text`. Renders from the SAME structs the JSON envelope
//! uses (plan §0.1 "human is a view, not the model") — never a separate
//! data path. `--time` mode selects the Time column display; machine
//! output is unaffected.
//!
//! Windows UTF-8: plain ASCII table borders (no box-drawing) so the stock
//! console never mojibakes (mirrors formatter.py's win32 workaround).

use twr_core::TimeMode;

/// Max tweet text chars unless --full-text.
pub const TRUNCATE_LEN: usize = 120;

pub fn truncate(text: &str, full: bool) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if full || flat.chars().count() <= TRUNCATE_LEN {
        return flat;
    }
    let cut: String = flat.chars().take(TRUNCATE_LEN - 1).collect();
    format!("{cut}…")
}

pub fn stats_line(m: &twr_model::Metrics) -> String {
    format!("L{} R{} r{} V{}", m.likes, m.retweets, m.replies, m.views)
}

/// Human table for a tweet list. `time_mode` from `--time`.
pub fn tweet_table(
    tweets: &[twr_model::Tweet],
    full_text: bool,
    time_mode: TimeMode,
    show_score: bool,
) -> String {
    let mut table = comfy_table::Table::new();
    table.load_preset(comfy_table::presets::ASCII_MARKDOWN);
    let mut header = vec!["#", "Author", "Tweet", "Stats", "Time"];
    if show_score {
        header.push("Score");
    }
    table.set_header(header);
    for (i, t) in tweets.iter().enumerate() {
        let author = format!("@{}", t.author.screen_name);
        let time = twr_core::display(&t.created_at, time_mode);
        let mut row = vec![
            i.to_string(),
            author,
            truncate(&t.text, full_text),
            stats_line(&t.metrics),
            time,
        ];
        if show_score {
            row.push(
                t.score
                    .map(|s| format!("{s:.1}"))
                    .unwrap_or_else(|| "-".into()),
            );
        }
        table.add_row(row);
    }
    table.to_string()
}

/// Human view for a single user profile.
pub fn user_card(u: &twr_model::UserProfile) -> String {
    format!(
        "@{} ({})\n{} followers · {} following · {} tweets\n{}",
        u.screen_name, u.name, u.followers_count, u.following_count, u.tweets_count, u.bio
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tweet(text: &str) -> twr_model::Tweet {
        twr_model::Tweet {
            id: "1".into(),
            text: text.into(),
            author: twr_model::Author {
                id: "u".into(),
                name: "A".into(),
                screen_name: "a".into(),
                profile_image_url: String::new(),
                verified: false,
            },
            metrics: twr_model::Metrics {
                likes: 5,
                retweets: 1,
                replies: 2,
                quotes: 0,
                views: 100,
                bookmarks: 0,
            },
            created_at: "Sat Mar 08 12:00:00 +0000 2026".into(),
            media: vec![],
            urls: vec![],
            is_retweet: false,
            lang: "en".into(),
            retweeted_by: None,
            quoted_tweet: None,
            score: Some(17.0),
            article_title: None,
            article_text: None,
            is_subscriber_only: false,
            is_promoted: false,
        }
    }

    #[test]
    fn truncates_long_text_unless_full() {
        let long = "x".repeat(200);
        assert!(truncate(&long, false).chars().count() <= 120);
        assert!(truncate(&long, false).ends_with('…'));
        assert_eq!(truncate(&long, true).len(), 200);
    }

    #[test]
    fn table_renders_same_structs_as_json() {
        let t = tweet("hello world");
        let out = tweet_table(std::slice::from_ref(&t), false, TimeMode::Relative, true);
        assert!(out.contains("@a"));
        assert!(out.contains("hello world"));
        assert!(out.contains("17.0"));
        assert!(out.contains("ago"));
        // No box-drawing (Windows-safe ASCII).
        assert!(!out.contains('─'));
    }

    #[test]
    fn absolute_time_column() {
        let t = tweet("hi");
        let out = tweet_table(std::slice::from_ref(&t), true, TimeMode::Absolute, false);
        assert!(out.contains("2026-03-08T12:00:00+00:00"));
    }
}
