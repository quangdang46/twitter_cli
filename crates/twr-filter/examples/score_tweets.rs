//! Score + filter parsed tweets (offline, no credentials).
//!
//! ```sh
//! cargo run --example score_tweets
//! ```

fn main() {
    let raw = std::fs::read_to_string("tests/fixtures/plain_tweet.json").expect("fixture");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("json");
    let tweet = twr_model::parse_tweet_result(&doc["input"], 0).expect("parses");
    let weights = twr_filter::Weights::default();
    println!("score: {}", twr_filter::score_tweet(&tweet, &weights));
    let cfg = twr_filter::FilterConfig::from_json(&serde_json::json!({"mode": "topN", "topN": 20}));
    println!(
        "kept: {}",
        twr_filter::filter_tweets(vec![tweet], &cfg).len()
    );
}
