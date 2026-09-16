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
    fn dm_send_is_write_only() {
        // Bead o1l.5.3: the highest-scrutiny write. Engagement MUST NOT
        // cover it (its spam profile has nothing in common with
        // like/retweet/follow/bookmark); only --policy write reaches it.
        assert!(!Policy::Engagement.allows("dm-send"), "dm-send");
        assert!(!Policy::ReadOnly.allows("dm-send"), "dm-send");
        assert!(Policy::Write.allows("dm-send"), "dm-send");
    }

    /// Bead o1l.5.4 §2 — full policy-boundary matrix for dm-send, verified
    /// live 2026-09-16 on the release binary: engagement → deny
    /// (`usage-policy-denied`, exit 2), read_only → deny (same envelope),
    /// write + --dry-run → allow (`{dry_run, operation: "dm-send",
    /// validation: "passed"}`, exit 0). The denial message must NAME the
    /// policy so an agent never confuses it with a budget denial.
    #[test]
    fn p54_dm_send_policy_matrix() {
        assert!(!Policy::ReadOnly.allows("dm-send"));
        assert!(!Policy::Engagement.allows("dm-send"));
        assert!(Policy::Write.allows("dm-send"));
        for p in [Policy::ReadOnly, Policy::Engagement] {
            let msg = p.denial("dm-send");
            assert!(msg.contains("dm-send"), "denial names the op: {msg}");
            assert!(msg.contains(p.as_str()), "denial names the policy: {msg}");
        }
        // Engagement's whitelist is a closed set: dm-send must never
        // silently join it via a future wildcard/match-all refactor.
        assert!(
            !Policy::Engagement.allows("DM-SEND"),
            "case-sensitive op names"
        );
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
