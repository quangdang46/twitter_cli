//! Policy gating: --policy read_only/engagement/write (bead twitter_cli-5o3.4.6).
//!
//! Checked BEFORE the §5.3 decision table so a read_only-scoped agent cannot
//! post even with --apply. Denials are exit 2 (usage/policy-denied).

/// Write policy tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    ReadOnly,
    Engagement,
    #[default]
    Write,
}

impl Policy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "read_only" | "readonly" | "read-only" => Some(Policy::ReadOnly),
            "engagement" => Some(Policy::Engagement),
            "write" => Some(Policy::Write),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Policy::ReadOnly => "read_only",
            Policy::Engagement => "engagement",
            Policy::Write => "write",
        }
    }

    /// Engagement-tier ops (like/rt/follow/bookmark + reverses). Post-family
    /// (post/reply/quote) and delete are NOT engagement.
    pub fn allows(&self, operation: &str) -> bool {
        match self {
            Policy::Write => true,
            Policy::ReadOnly => false,
            Policy::Engagement => matches!(
                operation,
                "like"
                    | "unlike"
                    | "retweet"
                    | "unretweet"
                    | "bookmark"
                    | "unbookmark"
                    | "follow"
                    | "unfollow"
            ),
        }
    }

    /// Denial message for a blocked write (exit 2).
    pub fn denial(&self, operation: &str) -> String {
        format!(
            "policy {} blocks {operation} (exit 2); change --policy or drop the write",
            self.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_gate_correctly() {
        assert!(!Policy::ReadOnly.allows("like"));
        assert!(!Policy::ReadOnly.allows("post"));
        assert!(Policy::Engagement.allows("like"));
        assert!(Policy::Engagement.allows("follow"));
        assert!(!Policy::Engagement.allows("post"));
        assert!(!Policy::Engagement.allows("delete"));
        assert!(Policy::Write.allows("post"));
        assert_eq!(Policy::parse("READ_ONLY"), Some(Policy::ReadOnly));
        assert_eq!(Policy::parse("nope"), None);
    }
}
