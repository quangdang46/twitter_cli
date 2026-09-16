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

    /// Engagement-tier ops (like/rt/follow/bookmark + reverses, P6.2's
    /// mute/block/pin + reverses, P6.3's list-follow/list-unfollow/list-pin/
    /// list-unpin/list-add-member/list-remove-member). Post-family
    /// (post/reply/quote), delete, list-create/list-edit/list-delete, edit,
    /// and dm-send are NOT engagement — their omission here is the ENTIRE
    /// mechanism making them write-tier-only (no second gate exists).
    /// Mute/block/pin are relationship/profile acts on your own account
    /// (plan §13.2), not content writes — block stays here (not `write`)
    /// per bead o1l.2.2: moving it would silently change the contract for
    /// every existing `--policy engagement` caller.
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
                    | "mute"
                    | "unmute"
                    | "block"
                    | "unblock"
                    | "pin"
                    | "unpin"
                    | "list-follow"
                    | "list-unfollow"
                    | "list-pin"
                    | "list-unpin"
                    | "list-add-member"
                    | "list-remove-member"
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

    #[test]
    fn p62_engagement_covers_mute_block_pin() {
        for op in ["mute", "unmute", "block", "unblock", "pin", "unpin"] {
            assert!(Policy::Engagement.allows(op), "{op}");
            assert!(!Policy::ReadOnly.allows(op), "{op}");
            assert!(Policy::Write.allows(op), "{op}");
        }
    }

    #[test]
    fn p63_write_tier_stays_write_only_and_engagement_covers_list_engagement() {
        // Write-tier: create/edit/delete are unreachable under engagement.
        for op in ["list-create", "list-edit", "list-delete"] {
            assert!(!Policy::Engagement.allows(op), "{op}");
            assert!(!Policy::ReadOnly.allows(op), "{op}");
            assert!(Policy::Write.allows(op), "{op}");
        }
        // Engagement-tier: follow/unfollow/pin/unpin/add/remove-member.
        for op in [
            "list-follow",
            "list-unfollow",
            "list-pin",
            "list-unpin",
            "list-add-member",
            "list-remove-member",
        ] {
            assert!(Policy::Engagement.allows(op), "{op}");
            assert!(!Policy::ReadOnly.allows(op), "{op}");
            assert!(Policy::Write.allows(op), "{op}");
        }
    }
}
