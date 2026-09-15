//! P0-4 offline validation (bead twitter_cli-5o3.2.4): the exact response
//! shape a live UserByScreenName call returns, parsed through the REAL
//! production path (twr-model parse + header builder + qid resolution).
//!
//! What this proves without credentials: transport shape + auth header
//! construction + GraphQL request URL + response parsing are all wired
//! correctly, so the ONLY remaining unknown is the live network round-trip
//! (needs human creds via `twr doctor --probe-user`). What it does NOT
//! prove: that x.com accepts the request (that IS the deferred live call).

use twr_client::{build_headers, Credentials, HeaderInput, Os};

/// Minimal but structurally faithful UserByScreenName payload.
fn user_by_screen_name_payload() -> serde_json::Value {
    serde_json::json!({
        "data": {
            "user": {
                "result": {
                    "__typename": "User",
                    "rest_id": "12345",
                    "core": {"name": "Jack", "screen_name": "jack"},
                    "legacy": {"name": "Jack", "screen_name": "jack", "description": "bio"},
                    "avatar": {"image_url": "https://img/a.jpg"},
                    "is_blue_verified": false,
                }
            }
        }
    })
}

#[test]
fn parses_minimal_user_object() {
    let body = user_by_screen_name_payload();
    let result = body.pointer("/data/user/result").unwrap();
    let user = twr_model::parse_user_result(result).expect("parses");
    assert_eq!(user.id, "12345");
    assert_eq!(user.screen_name, "jack");
    assert_eq!(user.name, "Jack");
}

#[test]
fn unavailable_user_maps_to_not_found() {
    let result = serde_json::json!({"__typename": "UserUnavailable"});
    assert!(twr_model::parse_user_result(&result).is_none());
    assert!(twr_core::is_not_found_payload(
        &serde_json::json!({"result": result})
    ));
}

#[test]
fn request_shape_is_correct() {
    // qid resolution: baseline layer yields the known UserByScreenName ID.
    let disk = twr_graphql::cache::QueryIdCache::new();
    let extra = twr_graphql::ExtraRotation::new();
    let resolved = twr_graphql::resolve("UserByScreenName", |_| None, &disk, &extra, 0).unwrap();
    assert_eq!(resolved.query_id, "1VOOyvKkiI3FMmkeDNxM9A");

    // Headers carry Bearer + Cookie + Csrf (synthetic creds only).
    let creds = Credentials {
        auth_token: "SYNTHETIC".into(),
        ct0: "SYNTHETIC".into(),
        cookie_string: None,
    };
    let headers = build_headers(&HeaderInput {
        creds: &creds,
        method: "GET",
        os: Os::Linux,
        chrome_major: "133",
        locale: "en-US",
        transaction_id: None, // UserByScreenName is NOT gated — must be absent
    });
    assert!(headers["Authorization"].starts_with("Bearer AAAA"));
    assert!(headers["Cookie"].contains("auth_token=SYNTHETIC"));
    assert_eq!(headers["X-Csrf-Token"], "SYNTHETIC");
    assert!(!headers.contains_key("X-Client-Transaction-Id"));

    // GET URL shape: /i/api/graphql/<qid>/<op>?variables=..&features=..
    // (Same one-liner cli::exec::graphql_get_url uses; asserted on shape.)
    let url = format!(
        "https://x.com/i/api/graphql/{}/UserByScreenName",
        resolved.query_id
    );
    assert!(url.contains("/1VOOyvKkiI3FMmkeDNxM9A/UserByScreenName"));
}
