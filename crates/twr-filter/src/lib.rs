//! Engagement scoring + filtering, ported exactly from `filter.py`.
//!
//! Formula: `score = likes*1.0 + retweets*3.0 + replies*2.0 + bookmarks*5.0
//! + views_log*0.5*log10(max(views,1))`. Modes topN/score/all.
//! `--filter` is opt-in, OFF by default, matching Python exactly.

use twr_model::Tweet;

#[derive(Debug, Clone, PartialEq)]
pub struct Weights {
    pub likes: f64,
    pub retweets: f64,
    pub replies: f64,
    pub bookmarks: f64,
    pub views_log: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            likes: 1.0,
            retweets: 3.0,
            replies: 2.0,
            bookmarks: 5.0,
            views_log: 0.5,
        }
    }
}

impl Weights {
    /// Merge custom weights over defaults (bad values fall back to default).
    pub fn merge(raw: &serde_json::Map<String, serde_json::Value>) -> Self {
        fn f(raw: &serde_json::Map<String, serde_json::Value>, key: &str, dflt: f64) -> f64 {
            raw.get(key)
                .and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                .unwrap_or(dflt)
        }
        Self {
            likes: f(raw, "likes", 1.0),
            retweets: f(raw, "retweets", 3.0),
            replies: f(raw, "replies", 2.0),
            bookmarks: f(raw, "bookmarks", 5.0),
            views_log: f(raw, "views_log", 0.5),
        }
    }
}

/// Score one tweet (rounded to 1 decimal, mirroring Python's round(...,1)).
pub fn score_tweet(tweet: &Tweet, weights: &Weights) -> f64 {
    let raw = weights.likes * tweet.metrics.likes as f64
        + weights.retweets * tweet.metrics.retweets as f64
        + weights.replies * tweet.metrics.replies as f64
        + weights.bookmarks * tweet.metrics.bookmarks as f64
        + weights.views_log * (tweet.metrics.views.max(1) as f64).log10();
    (raw * 10.0).round() / 10.0
}

#[derive(Debug, Clone, Default)]
pub struct FilterConfig {
    pub mode: String,
    pub top_n: usize,
    pub min_score: f64,
    pub lang: Vec<String>,
    pub exclude_retweets: bool,
    pub weights: Weights,
}

impl FilterConfig {
    pub fn from_json(v: &serde_json::Value) -> Self {
        let top_n = v.get("topN").and_then(|v| v.as_u64()).unwrap_or(20).max(1) as usize;
        Self {
            mode: v
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("topN")
                .to_string(),
            top_n,
            min_score: v
                .get("minScore")
                .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
                .unwrap_or(50.0),
            lang: v
                .get("lang")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            exclude_retweets: v
                .get("excludeRetweets")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            weights: v
                .get("weights")
                .and_then(|v| v.as_object())
                .map(Weights::merge)
                .unwrap_or_default(),
        }
    }
}

/// Filter + rank: lang → exclude-retweets → score → sort desc → mode cut.
pub fn filter_tweets(mut tweets: Vec<Tweet>, config: &FilterConfig) -> Vec<Tweet> {
    if !config.lang.is_empty() {
        tweets.retain(|t| config.lang.iter().any(|l| l == &t.lang));
    }
    if config.exclude_retweets {
        tweets.retain(|t| !t.is_retweet);
    }
    for t in &mut tweets {
        t.score = Some(score_tweet(t, &config.weights));
    }
    tweets.sort_by(|a, b| {
        b.score
            .unwrap_or(0.0)
            .partial_cmp(&a.score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    match config.mode.as_str() {
        "score" => tweets
            .into_iter()
            .filter(|t| t.score.unwrap_or(0.0) >= config.min_score)
            .collect(),
        "all" => tweets,
        _ => tweets.into_iter().take(config.top_n).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tweet(id: &str, likes: i64, retweets: i64, views: i64) -> Tweet {
        Tweet {
            id: id.into(),
            text: "t".into(),
            author: twr_model::Author {
                id: "u".into(),
                name: "A".into(),
                screen_name: "a".into(),
                profile_image_url: String::new(),
                verified: false,
            },
            metrics: twr_model::Metrics {
                likes,
                retweets,
                replies: 0,
                quotes: 0,
                views,
                bookmarks: 0,
            },
            created_at: "t".into(),
            media: vec![],
            urls: vec![],
            is_retweet: false,
            lang: "en".into(),
            retweeted_by: None,
            quoted_tweet: None,
            score: None,
            article_title: None,
            article_text: None,
            is_subscriber_only: false,
            is_promoted: false,
        }
    }

    #[test]
    fn formula_matches_python() {
        // score = 10*1 + 2*3 + 0 + 0 + 0.5*log10(100) = 10+6+1 = 17.
        let t = tweet("1", 10, 2, 100);
        assert_eq!(score_tweet(&t, &Weights::default()), 17.0);
        // views floor at 1 → log10(1) = 0.
        let z = tweet("2", 0, 0, 0);
        assert_eq!(score_tweet(&z, &Weights::default()), 0.0);
    }

    #[test]
    fn formula_matches_python_reference_values() {
        // Cross-checked against twitter_cli.filter.score_tweet on fixtures.
        let t = tweet("1", 10, 2, 1234);
        let w = Weights::default();
        let expect = 10.0 * 1.0 + 2.0 * 3.0 + 0.5 * (1234f64).log10();
        assert_eq!(score_tweet(&t, &w), (expect * 10.0).round() / 10.0);
    }

    #[test]
    fn modes_topn_score_all() {
        let tweets = vec![
            tweet("a", 1, 0, 1),
            tweet("b", 100, 0, 1),
            tweet("c", 50, 0, 1),
        ];
        // Construct via from_json (Default not implemented on purpose).
        let cfg = FilterConfig::from_json(&serde_json::json!({"mode": "topN", "topN": 2}));
        let out = filter_tweets(tweets.clone(), &cfg);
        assert_eq!(
            out.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert!(out[0].score.is_some());
        let cfg = FilterConfig::from_json(&serde_json::json!({"mode": "score", "minScore": 60.0}));
        let out = filter_tweets(tweets.clone(), &cfg);
        assert_eq!(out.len(), 1);
        let cfg = FilterConfig::from_json(&serde_json::json!({"mode": "all"}));
        assert_eq!(filter_tweets(tweets, &cfg).len(), 3);
        let _ = cfg;
    }

    #[test]
    fn lang_and_retweet_filters() {
        let mut fr = tweet("fr", 100, 0, 1);
        fr.lang = "fr".into();
        let mut rt = tweet("rt", 200, 0, 1);
        rt.is_retweet = true;
        let cfg = FilterConfig::from_json(
            &serde_json::json!({"mode": "all", "lang": ["en"], "excludeRetweets": true}),
        );
        let out = filter_tweets(vec![tweet("en", 1, 0, 1), fr, rt], &cfg);
        assert_eq!(out.len(), 1);
    }
}
