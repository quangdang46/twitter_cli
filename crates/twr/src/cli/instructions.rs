//! Per-operation timeline instruction extractors (live-validation fix).
//!
//! Every `_fetch_timeline` call passes `|_| None`, so live pages always
//! parsed to zero tweets. These mirror the Python `get_instructions`
//! lambdas exactly (deep-get paths per operation + documented fallbacks).

use serde_json::Value;

fn deep<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

fn as_instructions(v: Option<&Value>) -> Option<&Vec<Value>> {
    v?.as_array()
}

/// Extractor for one operation. Returns None for unknown ops (caller treats
/// as empty page rather than crashing).
pub fn for_operation(operation: &str) -> fn(&Value) -> Option<&Vec<Value>> {
    match operation {
        "HomeTimeline" | "HomeLatestTimeline" => |data| {
            as_instructions(deep(
                data,
                &["data", "home", "home_timeline_urt", "instructions"],
            ))
        },
        "Bookmarks" => |data| {
            as_instructions(
                deep(
                    data,
                    &["data", "bookmark_timeline", "timeline", "instructions"],
                )
                .or_else(|| {
                    deep(
                        data,
                        &["data", "bookmark_timeline_v2", "timeline", "instructions"],
                    )
                }),
            )
        },
        "BookmarkFolderTimeline" => |data| {
            as_instructions(deep(
                data,
                &[
                    "data",
                    "bookmark_collection_timeline",
                    "timeline",
                    "instructions",
                ],
            ))
        },
        "UserTweets" | "Likes" => |data| {
            as_instructions(
                deep(
                    data,
                    &[
                        "data",
                        "user",
                        "result",
                        "timeline",
                        "timeline",
                        "instructions",
                    ],
                )
                .or_else(|| {
                    deep(
                        data,
                        &[
                            "data",
                            "user",
                            "result",
                            "timeline_v2",
                            "timeline",
                            "instructions",
                        ],
                    )
                }),
            )
        },
        "SearchTimeline" => |data| {
            as_instructions(deep(
                data,
                &[
                    "data",
                    "search_by_raw_query",
                    "search_timeline",
                    "timeline",
                    "instructions",
                ],
            ))
        },
        "TweetDetail" => |data| {
            as_instructions(
                deep(
                    data,
                    &["data", "tweetResult", "result", "timeline", "instructions"],
                )
                .or_else(|| {
                    deep(
                        data,
                        &[
                            "data",
                            "threaded_conversation_with_injections_v2",
                            "instructions",
                        ],
                    )
                }),
            )
        },
        "ListLatestTweetsTimeline" => |data| {
            as_instructions(deep(
                data,
                &[
                    "data",
                    "list",
                    "tweets_timeline",
                    "timeline",
                    "instructions",
                ],
            ))
        },
        "ListOwnerships" | "ListMemberships" => |data| {
            as_instructions(deep(
                data,
                &[
                    "data",
                    "user",
                    "result",
                    "timeline",
                    "timeline",
                    "instructions",
                ],
            ))
        },
        "ListMembers" => |data| {
            as_instructions(deep(
                data,
                &[
                    "data",
                    "list",
                    "members_timeline",
                    "timeline",
                    "instructions",
                ],
            ))
        },
        "Followers" | "Following" => |data| {
            as_instructions(deep(
                data,
                &[
                    "data",
                    "user",
                    "result",
                    "timeline",
                    "timeline",
                    "instructions",
                ],
            ))
        },
        _ => |_| None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn home_and_search_paths() {
        let home = json!({"data": {"home": {"home_timeline_urt": {"instructions": [{"a": 1}]}}}});
        assert_eq!(for_operation("HomeTimeline")(&home).unwrap().len(), 1);
        let search = json!({"data": {"search_by_raw_query": {"search_timeline": {"timeline": {"instructions": []}}}}});
        assert!(for_operation("SearchTimeline")(&search).unwrap().is_empty());
        assert!(for_operation("Nope")(&home).is_none());
    }

    #[test]
    fn list_ops_hit_their_instruction_paths() {
        let owned = json!({"data": {"user": {"result": {"timeline": {"timeline": {"instructions": [{"a": 1}]}}}}}});
        assert_eq!(for_operation("ListOwnerships")(&owned).unwrap().len(), 1);
        assert_eq!(for_operation("ListMemberships")(&owned).unwrap().len(), 1);
        let members =
            json!({"data": {"list": {"members_timeline": {"timeline": {"instructions": []}}}}});
        assert!(for_operation("ListMembers")(&members).unwrap().is_empty());
    }

    #[test]
    fn user_falls_back_to_timeline_v2() {
        let v2 = json!({"data": {"user": {"result": {"timeline_v2": {"timeline": {"instructions": [{"b": 2}]}}}}}});
        assert_eq!(for_operation("UserTweets")(&v2).unwrap().len(), 1);
    }
}
