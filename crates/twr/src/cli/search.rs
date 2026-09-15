//! Search query builder: operator flags → X search string + variables.
//!
//! Mirrors the Python `search.py` query construction: `-t Top|Latest|Photos|
//! Videos` product, `--from/--to/--lang/--since/--until`, repeatable
//! `--has/--exclude`, `--min-likes/--min-retweets`.

/// Search product tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchProduct {
    #[default]
    Top,
    Latest,
    Photos,
    Videos,
}

impl SearchProduct {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "top" => Some(SearchProduct::Top),
            "latest" => Some(SearchProduct::Latest),
            "photos" => Some(SearchProduct::Photos),
            "videos" => Some(SearchProduct::Videos),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SearchProduct::Top => "Top",
            SearchProduct::Latest => "Latest",
            SearchProduct::Photos => "Photos",
            SearchProduct::Videos => "Videos",
        }
    }
}

/// All search-shaping flags in one struct.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub query: String,
    pub product: SearchProduct,
    pub from: Option<String>,
    pub to: Option<String>,
    pub lang: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub has: Vec<String>,
    pub exclude: Vec<String>,
    pub min_likes: Option<u64>,
    pub min_retweets: Option<u64>,
}

impl SearchQuery {
    /// Render the raw X search string (`q` variable).
    pub fn raw_query(&self) -> String {
        let mut parts = vec![self.query.clone()];
        if let Some(from) = &self.from {
            parts.push(format!("from:{from}"));
        }
        if let Some(to) = &self.to {
            parts.push(format!("to:{to}"));
        }
        if let Some(lang) = &self.lang {
            parts.push(format!("lang:{lang}"));
        }
        if let Some(since) = &self.since {
            parts.push(format!("since:{since}"));
        }
        if let Some(until) = &self.until {
            parts.push(format!("until:{until}"));
        }
        for h in &self.has {
            parts.push(format!("has:{h}"));
        }
        for e in &self.exclude {
            parts.push(format!("-{e}"));
        }
        if let Some(n) = self.min_likes {
            parts.push(format!("min_faves:{n}"));
        }
        if let Some(n) = self.min_retweets {
            parts.push(format!("min_retweets:{n}"));
        }
        parts
            .into_iter()
            .filter(|p| !p.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_parses_case_insensitively() {
        assert_eq!(SearchProduct::parse("latest"), Some(SearchProduct::Latest));
        assert_eq!(SearchProduct::parse("PHOTOS"), Some(SearchProduct::Photos));
        assert_eq!(SearchProduct::parse("nope"), None);
    }

    #[test]
    fn raw_query_combines_operators() {
        let q = SearchQuery {
            query: "rust".into(),
            product: SearchProduct::Latest,
            from: Some("alice".into()),
            min_likes: Some(10),
            has: vec!["images".into()],
            exclude: vec!["replies".into()],
            ..Default::default()
        };
        assert_eq!(
            q.raw_query(),
            "rust from:alice has:images -replies min_faves:10"
        );
        assert_eq!(q.product.as_str(), "Latest");
    }
}
