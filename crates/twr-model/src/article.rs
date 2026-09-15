//! Twitter Article (long-form) extraction: Draft.js content blocks -> Markdown.
//! Ported from `parser.py`'s `_parse_article` and its helpers, verbatim logic.

use crate::dget;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// `article_title`/`article_text` pair `_parse_article` returns as a dict in
/// Python; both are `None` when the tweet isn't an article at all.
pub struct ArticleFields {
    pub title: Option<String>,
    pub text: Option<String>,
}

const IMAGE_URL_KEYS: &[&str] = &[
    "original_img_url",
    "originalImgUrl",
    "original_url",
    "originalUrl",
    "media_url_https",
    "mediaUrlHttps",
    "media_url",
    "mediaUrl",
    "url",
    "src",
    "uri",
];

fn looks_like_image_url(candidate: &str) -> bool {
    let lowered = candidate.to_lowercase();
    lowered.starts_with("https://pbs.twimg.com/")
        || [".jpg", ".jpeg", ".png", ".gif", ".webp"]
            .iter()
            .any(|ext| lowered.ends_with(ext))
        || [".jpg?", ".jpeg?", ".png?", ".gif?", ".webp?"]
            .iter()
            .any(|ext| lowered.contains(ext))
}

/// Best-effort recursive extraction of the original image URL from article
/// entity data (mirrors `_find_article_image_url`'s DFS over dict/list).
pub fn find_article_image_url(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in IMAGE_URL_KEYS {
                if let Some(Value::String(s)) = map.get(*key) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() && looks_like_image_url(trimmed) {
                        return Some(trimmed.to_string());
                    }
                }
            }
            for nested in map.values() {
                if let Some(found) = find_article_image_url(nested) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_article_image_url),
        _ => None,
    }
}

const CAPTION_KEYS: &[&str] = &["caption", "alt", "alt_text", "altText", "title", "name"];

/// Mirrors `_find_article_caption`.
pub fn find_article_caption(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in CAPTION_KEYS {
                if let Some(Value::String(s)) = map.get(*key) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
            for nested in map.values() {
                if let Some(found) = find_article_caption(nested) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_article_caption),
        _ => None,
    }
}

/// Normalize Draft.js `entityMap`, which may arrive as an object keyed by
/// entity id, or as `[{key, value}, ...]` (mirrors `_normalize_article_entity_map`).
pub fn normalize_entity_map(entity_map: &Value) -> HashMap<String, Value> {
    match entity_map {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Array(items) => {
            let mut out = HashMap::new();
            for item in items {
                let Value::Object(obj) = item else { continue };
                let (Some(key), Some(value)) = (obj.get("key"), obj.get("value")) else {
                    continue;
                };
                let key_str = match key {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.insert(key_str, value.clone());
            }
            out
        }
        _ => HashMap::new(),
    }
}

/// Map article media ids/keys to original image URLs, for atomic entities
/// that reference media only by id (mirrors `_extract_article_media_url_map`).
pub fn extract_media_url_map(article_results: &Map<String, Value>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut candidates: Vec<&Value> = Vec::new();
    if let Some(cover) = article_results.get("cover_media") {
        if !cover.is_null() {
            candidates.push(cover);
        }
    }
    if let Some(Value::Array(entities)) = article_results.get("media_entities") {
        candidates.extend(entities.iter());
    }

    for media in candidates {
        let Value::Object(media_obj) = media else {
            continue;
        };
        let media_info = media_obj.get("media_info");
        let image_url = media_info
            .and_then(find_article_image_url)
            .or_else(|| find_article_image_url(media));
        let Some(image_url) = image_url else { continue };
        for key in ["media_id", "media_key", "id"] {
            if let Some(Value::String(id)) = media_obj.get(key) {
                if !id.is_empty() {
                    out.insert(id.clone(), image_url.clone());
                }
            }
        }
    }
    out
}

fn entity_ranges(block: &Value) -> Vec<&Value> {
    match block.get("entityRanges") {
        Some(Value::Array(items)) => items.iter().collect(),
        _ => Vec::new(),
    }
}

fn entity_for<'a>(range: &Value, entity_map: &'a HashMap<String, Value>) -> Option<&'a Value> {
    let key = range.get("key")?;
    let key_str = match key {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    entity_map.get(&key_str)
}

/// Extract embedded markdown/code payloads from atomic Draft.js entities
/// (mirrors `_extract_atomic_markdown`).
pub fn extract_atomic_markdown(block: &Value, entity_map: &HashMap<String, Value>) -> Vec<String> {
    let mut parts = Vec::new();
    for range in entity_ranges(block) {
        let Some(entity) = entity_for(range, entity_map) else {
            continue;
        };
        let entity_type = entity
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_uppercase();
        if entity_type != "MARKDOWN" {
            continue;
        }
        if let Some(Value::String(md)) = dget!(entity, "data", "markdown") {
            let trimmed = md.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    parts
}

/// Render a Draft.js text block, converting inline LINK entities to Markdown
/// links (mirrors `_render_article_text_block`, including its right-to-left
/// splice order so earlier offsets stay valid while later ones are rewritten).
pub fn render_text_block(block: &Value, entity_map: &HashMap<String, Value>) -> String {
    let text = match block.get("text") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => return String::new(),
    };
    let ranges = entity_ranges(block);
    if ranges.is_empty() {
        return text;
    }

    // Collect (offset, length, url), skip anything not a LINK with a usable url.
    let mut link_ranges: Vec<(i64, i64, String)> = Vec::new();
    for range in &ranges {
        let Some(entity) = entity_for(range, entity_map) else {
            continue;
        };
        let entity_type = entity
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_uppercase();
        if entity_type != "LINK" {
            continue;
        }
        let (Some(offset), Some(length)) = (
            range.get("offset").and_then(Value::as_i64),
            range.get("length").and_then(Value::as_i64),
        ) else {
            continue;
        };
        if length <= 0 {
            continue;
        }
        let Some(Value::String(url)) = dget!(entity, "data", "url") else {
            continue;
        };
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        link_ranges.push((offset, length, url.to_string()));
    }

    // Python: `for offset, length, url in sorted(ranges, reverse=True)` --
    // tuple sort descending, so higher offsets are spliced first.
    link_ranges.sort_by(|a, b| b.cmp(a));

    // Operate over char indices (Draft.js offsets are UTF-16 code units in
    // the real client, but this port -- like the Python original -- assumes
    // simple text and indexes by Rust `char`s; non-BMP/CJK-heavy content may
    // drift, matching upstream's own limitation rather than introducing a new one).
    let mut chars: Vec<char> = text.chars().collect();
    for (offset, length, url) in link_ranges {
        let (offset, length) = (offset as usize, length as usize);
        if offset + length > chars.len() {
            continue;
        }
        let label: String = chars[offset..offset + length].iter().collect();
        if label.is_empty() {
            continue;
        }
        let safe_label = label.replace('[', "\\[").replace(']', "\\]");
        let safe_url = url.replace(')', "%29");
        let replacement = format!("[{safe_label}]({safe_url})");
        let replacement_chars: Vec<char> = replacement.chars().collect();
        chars.splice(offset..offset + length, replacement_chars);
    }
    chars.into_iter().collect()
}

/// Convert atomic Draft.js image entities to Markdown image lines (mirrors
/// `_extract_article_images`).
pub fn extract_images(
    block: &Value,
    entity_map: &HashMap<String, Value>,
    media_url_map: &HashMap<String, String>,
) -> Vec<String> {
    let mut parts = Vec::new();
    for range in entity_ranges(block) {
        let Some(entity) = entity_for(range, entity_map) else {
            continue;
        };
        let mut image_url = find_article_image_url(entity);
        if image_url.is_none() {
            if let Some(Value::Array(items)) = dget!(entity, "data", "mediaItems") {
                for item in items {
                    if let Some(Value::String(media_id)) = item.get("mediaId") {
                        if let Some(url) = media_url_map.get(media_id) {
                            image_url = Some(url.clone());
                            break;
                        }
                    }
                }
            }
        }
        let Some(image_url) = image_url else { continue };
        let caption = find_article_caption(entity).unwrap_or_default();
        parts.push(format!("![{caption}]({image_url})"));
    }
    parts
}

/// Top-level article extraction, mirrors `_parse_article` exactly, including
/// the ordered-list-item counter reset behavior.
pub fn parse_article(tweet_data: &Value) -> ArticleFields {
    let Some(Value::Object(article_results)) =
        dget!(tweet_data, "article", "article_results", "result")
    else {
        return ArticleFields {
            title: None,
            text: None,
        };
    };

    let title = article_results
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string);
    let content_state = article_results.get("content_state");
    let blocks = match content_state.and_then(|c| c.get("blocks")) {
        Some(Value::Array(blocks)) if !blocks.is_empty() => blocks,
        _ => return ArticleFields { title, text: None },
    };

    let entity_map = content_state
        .and_then(|c| c.get("entityMap"))
        .map(normalize_entity_map)
        .unwrap_or_default();
    let media_url_map = extract_media_url_map(article_results);

    let mut parts: Vec<String> = Vec::new();
    let mut ordered_counter = 0i64;
    for block in blocks {
        let block_type = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unstyled");
        if block_type == "atomic" {
            parts.extend(extract_atomic_markdown(block, &entity_map));
            parts.extend(extract_images(block, &entity_map, &media_url_map));
            ordered_counter = 0;
            continue;
        }
        let text = render_text_block(block, &entity_map);
        if text.is_empty() {
            continue;
        }
        if block_type != "ordered-list-item" {
            ordered_counter = 0;
        }
        match block_type {
            "header-one" => parts.push(format!("# {text}")),
            "header-two" => parts.push(format!("## {text}")),
            "header-three" => parts.push(format!("### {text}")),
            "blockquote" => parts.push(format!("> {text}")),
            "unordered-list-item" => parts.push(format!("- {text}")),
            "ordered-list-item" => {
                ordered_counter += 1;
                parts.push(format!("{ordered_counter}. {text}"));
            }
            "code-block" => parts.push(format!("```\n{text}\n```")),
            _ => parts.push(text),
        }
    }

    let text = if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    };
    ArticleFields { title, text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn article_tweet(blocks: serde_json::Value) -> serde_json::Value {
        json!({
            "article": {
                "article_results": {
                    "result": {
                        "title": "My Title",
                        "content_state": {
                            "blocks": blocks,
                            "entityMap": {},
                        },
                    },
                },
            },
        })
    }

    fn text_block(text: &str, kind: &str) -> serde_json::Value {
        json!({"text": text, "type": kind, "entityRanges": [], "inlineStyleRanges": []})
    }

    #[test]
    fn renders_headings_quote_lists_code() {
        let blocks = json!([
            text_block("H1", "header-one"),
            text_block("H2", "header-two"),
            text_block("H3", "header-three"),
            text_block("quoted", "blockquote"),
            text_block("u1", "unordered-list-item"),
            text_block("o1", "ordered-list-item"),
            text_block("o2", "ordered-list-item"),
            text_block("code!", "code-block"),
            text_block("plain", "unstyled"),
        ]);
        let out = parse_article(&article_tweet(blocks));
        assert_eq!(out.title.as_deref(), Some("My Title"));
        let text = out.text.unwrap();
        assert!(text.contains("# H1"));
        assert!(text.contains("## H2"));
        assert!(text.contains("### H3"));
        assert!(text.contains("> quoted"));
        assert!(text.contains("- u1"));
        assert!(text.contains("1. o1"));
        assert!(text.contains("2. o2"));
        assert!(text.contains("```\ncode!\n```"));
        assert!(text.contains("plain"));
    }

    #[test]
    fn missing_article_yields_nones() {
        let out = parse_article(&json!({}));
        assert!(out.title.is_none() && out.text.is_none());
    }
}
