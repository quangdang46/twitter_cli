//! Extract the four `loading-x-anim-N` SVG frames out of x.com's home page HTML.
//!
//! Ported from `agentic_x.transaction._FrameParser` (an `html.parser.HTMLParser`
//! subclass). Rather than pull in a full HTML5 parser dependency for this one
//! narrow, well-formed structure, this replicates the same small state machine
//! (active frame id + child depth) over a lightweight tag tokenizer.

use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(/)?([a-zA-Z][-a-zA-Z0-9]*)([^>]*)>").unwrap());
static ATTR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*=\s*"([^"]*)""#).unwrap());
static FRAME_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^loading-x-anim-(\d+)$").unwrap());

struct FrameParser {
    frames: BTreeMap<u32, Vec<String>>,
    active: Option<u32>,
    depth: i32,
}

impl FrameParser {
    fn new() -> Self {
        Self {
            frames: BTreeMap::new(),
            active: None,
            depth: 0,
        }
    }

    fn attr<'a>(rest: &'a str, name: &str) -> Option<&'a str> {
        ATTR_RE
            .captures_iter(rest)
            .find(|c| &c[1] == name)
            .map(|c| {
                // SAFETY-free: we can't return a borrow tied to a temp match; re-slice from rest.
                let m = c.get(2).unwrap();
                &rest[m.start()..m.end()]
            })
    }

    fn handle_start(&mut self, rest: &str) {
        match self.active {
            None => {
                if let Some(id) = Self::attr(rest, "id") {
                    if let Some(caps) = FRAME_ID_RE.captures(id) {
                        let n: u32 = caps[1].parse().unwrap();
                        self.active = Some(n);
                        self.frames.insert(n, Vec::new());
                        self.depth = 0;
                    }
                }
            }
            Some(active) => {
                self.depth += 1;
                if self.depth == 2 {
                    let d = Self::attr(rest, "d").unwrap_or("").to_string();
                    self.frames.get_mut(&active).unwrap().push(d);
                }
            }
        }
    }

    fn handle_end(&mut self) {
        if self.active.is_none() {
            return;
        }
        if self.depth == 0 {
            self.active = None;
        } else {
            self.depth -= 1;
        }
    }
}

/// Parse the four loading-animation frames out of x.com's HTML.
///
/// Returns, per frame index, the `d` attributes of the element children of
/// the first element child of that `loading-x-anim-N` node (mirroring
/// upstream's `list(list(frame.children)[0].children)[1].get("d")` access).
pub fn extract_frame_paths(html: &str) -> BTreeMap<u32, Vec<String>> {
    let mut parser = FrameParser::new();
    for caps in TAG_RE.captures_iter(html) {
        let is_end = caps.get(1).is_some();
        let rest = &caps[3];
        let self_close = rest.trim_end().ends_with('/');
        if is_end {
            parser.handle_end();
        } else {
            parser.handle_start(rest);
            if self_close {
                parser.handle_end();
            }
        }
    }
    parser.frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segments() -> String {
        (0..12)
            .map(|row| {
                (0..11)
                    .map(|n| ((row * 11 + n) % 256).to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("C")
    }

    fn path_d() -> String {
        format!("M0 0 C 0 {}", segments())
    }

    fn page() -> String {
        let d = path_d();
        let frames: String = (0..4)
            .map(|i| {
                format!(
                    r#"<svg id="loading-x-anim-{i}"><g><path d="M1 1"/><path d="{d}"/></g></svg>"#
                )
            })
            .collect();
        format!(
            r#"<html><head><meta name="twitter-site-verification" content="AAAA"/></head><body>{frames}<script>e={{,59924:"ondemand.s",59924:"deadbeef"}}</script></body></html>"#
        )
    }

    #[test]
    fn reads_all_four_frames() {
        let frames = extract_frame_paths(&page());
        let ids: Vec<u32> = frames.keys().copied().collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
        assert_eq!(frames[&1], vec!["M1 1".to_string(), path_d()]);
    }

    #[test]
    fn ignores_unrelated_elements() {
        let html = format!(
            r#"<div id="not-a-frame"><g><path d="X"/></g></div>{}"#,
            page()
        );
        let frames = extract_frame_paths(&html);
        let ids: Vec<u32> = frames.keys().copied().collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }
}
