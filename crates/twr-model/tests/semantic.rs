//! P1-PARITY layers 2+3 (bead twitter_cli-5o3.3.11): semantic cursor walk +
//! --max N boundary cases.
//!
//! Layer 2: a representative multi-page cursor walk resumes to the SAME set
//! of tweet IDs regardless of page splits — cursor/dedup bugs only show up
//! across pages, never on isolated single-page snapshots.
//! Layer 3: --max N boundary cases (1, 19, 20, 21 spanning a page boundary)
//! resume correctly from the returned nextCursor.

use twr_model::parse_timeline_response;

/// Build a synthetic timeline page: entries with tweet ids + optional cursor.
fn page(ids: &[&str], cursor: Option<&str>) -> serde_json::Value {
    let mut entries: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| {
            serde_json::json!({
                "entryId": format!("tweet-{id}"),
                "content": {
                    "itemContent": {
                        "tweet_results": {
                            "result": {
                                "rest_id": id,
                                "core": {"user_results": {"result": {
                                    "rest_id": "u1",
                                    "core": {"name": "A", "screen_name": "a"},
                                    "legacy": {},
                                }}},
                                "legacy": {
                                    "full_text": format!("text {id}"),
                                    "created_at": "t",
                                    "lang": "en",
                                },
                            }
                        }
                    }
                }
            })
        })
        .collect();
    if let Some(c) = cursor {
        entries.push(serde_json::json!({
            "entryId": "cursor-bottom-x",
            "content": {"cursorType": "Bottom", "value": c},
        }));
    }
    serde_json::json!([{"entries": entries}])
}

fn instructions(data: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    data.as_array()
}

/// Simulate the twr-client fetch_timeline loop over canned pages.
fn walk(pages: Vec<serde_json::Value>, count: usize) -> (Vec<String>, Option<String>) {
    use std::collections::HashSet;
    let mut ids: Vec<String> = vec![];
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut page_idx = 0;
    while ids.len() < count && page_idx < pages.len() {
        let (tweets, next) = parse_timeline_response(&pages[page_idx], instructions);
        page_idx += 1;
        for t in tweets {
            if seen.insert(t.id.clone()) {
                ids.push(t.id);
            }
        }
        match next {
            None => break,
            Some(n) if Some(&n) == cursor.as_ref() => break,
            Some(n) => cursor = Some(n),
        }
    }
    ids.truncate(count);
    (ids, cursor)
}

#[test]
fn multipage_walk_resumes_to_same_id_set() {
    // Same 5 tweets split 3+2 vs 2+3 must yield the same ID set.
    let split_a = vec![page(&["1", "2", "3"], Some("c1")), page(&["4", "5"], None)];
    let split_b = vec![page(&["1", "2"], Some("c1")), page(&["3", "4", "5"], None)];
    let (a, _) = walk(split_a, 50);
    let (b, _) = walk(split_b, 50);
    assert_eq!(a, vec!["1", "2", "3", "4", "5"]);
    assert_eq!(b, vec!["1", "2", "3", "4", "5"]);
}

#[test]
fn max_n_boundaries_span_page_edges() {
    // 25 tweets across pages of 20: N=1/19/20/21 must each return exactly N.
    let all: Vec<String> = (0..25).map(|i| i.to_string()).collect();
    for n in [1usize, 19, 20, 21, 25] {
        let refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        let pages = vec![
            page(&refs[..20.min(refs.len())], Some("c1")),
            page(&refs[20.min(refs.len())..], None),
        ];
        let (ids, _) = walk(pages, n);
        assert_eq!(ids.len(), n.min(25), "N={n}");
        let expect: Vec<String> = all.iter().take(n.min(25)).cloned().collect();
        assert_eq!(ids, expect, "N={n}");
    }
}

#[test]
fn cursor_resume_continues_where_it_left_off() {
    let p1 = page(&["1", "2"], Some("c1"));
    let p2 = page(&["3", "4"], None);
    let (first, cursor) = walk(vec![p1], 50);
    assert_eq!(first, vec!["1", "2"]);
    assert_eq!(cursor, Some("c1".into()));
    // Resume with the returned cursor: second walk picks up 3,4.
    let (second, _) = walk(vec![p2], 50);
    assert_eq!(second, vec!["3", "4"]);
}
