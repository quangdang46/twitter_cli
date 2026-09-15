#!/usr/bin/env python3
"""Fixture capture harness (bead twitter_cli-5o3.3.10).

Runs the ORIGINAL public-clis/twitter-cli parser (offline — no network, no
credentials) over synthetic GraphQL payloads covering every parser edge case
in plan §1.2, and saves both the raw input payload and the Python-parsed
output under tests/fixtures/*.json.

These are the ground truth for P1-PARITY (bead 3.3.11): the Rust twr-model
parser must produce field-for-field identical domain structs.

Usage: python3 tests/capture_fixtures.py
Requires: pip install twitter-cli (pinned by CI or manual step).
"""

import json
import os
import sys

try:
    from twitter_cli.parser import parse_tweet_result, parse_user_result
except ImportError:
    print("twitter-cli not installed: pip install twitter-cli", file=sys.stderr)
    sys.exit(2)

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")


def user_result(rest_id="u1", screen_name="ada", name="Ada Lovelace"):
    return {
        "rest_id": rest_id,
        "core": {"name": name, "screen_name": screen_name},
        "legacy": {"name": name, "screen_name": screen_name},
        "is_blue_verified": True,
        "avatar": {"image_url": "https://img/x.jpg"},
    }


def tweet_result(tid="100", text="hello world", **over):
    base = {
        "rest_id": tid,
        "core": {"user_results": {"result": user_result()}},
        "legacy": {
            "full_text": text,
            "favorite_count": 10,
            "retweet_count": 2,
            "reply_count": 1,
            "quote_count": 0,
            "bookmark_count": 3,
            "created_at": "Mon Jan 01 00:00:00 +0000 2024",
            "lang": "en",
            "entities": {"urls": []},
        },
        "views": {"count": "1,234"},
    }
    base.update(over)
    return base


def unwrap_visibility(tweet):
    return {
        "__typename": "TweetWithVisibilityResults",
        "tweet": tweet,
    }


def retweet_shell(inner, by="bob"):
    shell = tweet_result(tid="shell1", text="RT placeholder")
    shell["legacy"]["retweeted_status_result"] = {"result": inner}
    shell["core"]["user_results"]["result"]["core"]["screen_name"] = by
    shell["core"]["user_results"]["result"]["legacy"]["screen_name"] = by
    return shell


def quote_shell(inner):
    shell = tweet_result(tid="q1", text="quoting")
    shell["quoted_status_result"] = {"result": inner}
    return shell


def photo_media_tweet():
    t = tweet_result()
    t["legacy"]["extended_entities"] = {
        "media": [
            {
                "type": "photo",
                "media_url_https": "https://img/photo.jpg",
                "original_info": {"width": 100, "height": 200},
            }
        ]
    }
    return t


def video_media_tweet():
    t = tweet_result()
    t["legacy"]["extended_entities"] = {
        "media": [
            {
                "type": "video",
                "original_info": {"width": 640, "height": 360},
                "video_info": {
                    "variants": [
                        {"content_type": "video/mp4", "bitrate": 100, "url": "https://v/low.mp4"},
                        {"content_type": "video/mp4", "bitrate": 900, "url": "https://v/hi.mp4"},
                        {"content_type": "application/x-mpegURL", "url": "https://v/pl.m3u8"},
                    ]
                },
            }
        ]
    }
    return t


def note_tweet_long():
    t = tweet_result(text="truncated…")
    t["note_tweet"] = {"note_tweet_results": {"result": {"text": "the FULL long text"}}}
    return t


CASES = {
    "plain_tweet": tweet_result(),
    "tombstone": {"__typename": "TweetTombstone"},
    "visibility_wrapped": unwrap_visibility(tweet_result(tid="200", text="gated")),
    "retweet": retweet_shell(tweet_result(tid="orig9", text="original text")),
    "quote_tweet": quote_shell(tweet_result(tid="inner7", text="inner text")),
    "photo_media": photo_media_tweet(),
    "video_media": video_media_tweet(),
    "note_tweet": note_tweet_long(),
    "user": user_result(),
    "user_unavailable": {"__typename": "UserUnavailable", "rest_id": ""},
}


def main():
    os.makedirs(FIXTURES, exist_ok=True)
    import dataclasses

    print(f"capturing {len(CASES)} fixtures -> {FIXTURES}")
    for name, payload in CASES.items():
        if name.startswith("user"):
            parsed = parse_user_result(payload)
        else:
            parsed = parse_tweet_result(payload)
        doc = {
            "name": name,
            "input": payload,
            "python_output": (
                dataclasses.asdict(parsed) if parsed is not None else None
            ),
        }
        path = os.path.join(FIXTURES, f"{name}.json")
        with open(path, "w") as f:
            json.dump(doc, f, indent=2)
        print(f"  {name}.json parsed={'null' if parsed is None else 'ok'}")
    print("done — twr-model must deep-equal python_output field-for-field (bead 3.3.11)")


if __name__ == "__main__":
    main()
