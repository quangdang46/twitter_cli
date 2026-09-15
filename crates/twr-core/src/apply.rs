//! Write-safety decision table (plan §5.3, bead twitter_cli-5o3.4.1).
//!
//! Uniform mechanism for EVERY write command. Five-row truth table:
//! - apply=no,  dry_run=no,  TTY          -> prompt; yes=execute, no/EOF=cancel note (exit 0)
//! - apply=no,  dry_run=no,  no-interactive -> exit 2 (ambiguous by design)
//! - apply=no,  dry_run=yes, either       -> DryRun preview {dry_run:true,
//!   operation, validation:"passed"} (NEVER "would_succeed"), exit 0, no network
//! - apply=yes, dry_run=no,  either       -> execute for real
//! - apply=yes, dry_run=yes, either       -> exit 2 (mutually exclusive)

/// Inputs to the decision.
pub struct ApplyInput {
    pub apply: bool,
    pub dry_run: bool,
    pub no_interactive: bool,
    pub stdin_is_tty: bool,
}

/// What the caller should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run the DryRun preview, exit 0, touch no network.
    Preview,
    /// Execute for real.
    Execute,
    /// Prompt on the TTY; the bool is (execute_on_yes).
    Prompt,
    /// Usage error (exit 2) with this message.
    Deny(String),
    /// Cancelled at the prompt (exit 0 with a dry_run-shaped note).
    Cancelled,
}

pub fn decide(input: &ApplyInput) -> Decision {
    if input.apply && input.dry_run {
        return Decision::Deny("--apply and --dry-run are mutually exclusive".into());
    }
    if input.apply {
        return Decision::Execute;
    }
    if input.dry_run {
        return Decision::Preview;
    }
    if input.no_interactive || !input.stdin_is_tty {
        // Non-TTY without --apply/--dry-run behaves like --no-interactive:
        // ambiguous by design, exit 2 (never prompt a pipe).
        return Decision::Deny(
            "ambiguous invocation: add --apply to execute or --dry-run to preview".into(),
        );
    }
    Decision::Prompt
}

/// DryRun preview envelope data. `validation` is exactly "passed" — never
/// "would_succeed" (no guarantee a later real call succeeds).
pub fn dry_run_data(operation: &str) -> serde_json::Value {
    serde_json::json!({"dry_run": true, "operation": operation, "validation": "passed"})
}

/// Cancellation note (prompt answered no / EOF): dry_run-shaped, exit 0.
pub fn cancelled_data(operation: &str) -> serde_json::Value {
    serde_json::json!({"dry_run": true, "operation": operation, "validation": "passed", "cancelled": true})
}

/// Exact denial message for the ambiguous case (discord-cli pattern).
pub fn ambiguous_message(action: &str, target: &str) -> String {
    format!("This will {action} \"{target}\". Add --confirm to proceed.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_executes_regardless_of_interactive() {
        for (ni, tty) in [(false, true), (true, true), (true, false)] {
            assert_eq!(
                decide(&ApplyInput {
                    apply: true,
                    dry_run: false,
                    no_interactive: ni,
                    stdin_is_tty: tty
                }),
                Decision::Execute
            );
        }
    }

    #[test]
    fn dry_run_previews_without_network() {
        assert_eq!(
            decide(&ApplyInput {
                apply: false,
                dry_run: true,
                no_interactive: false,
                stdin_is_tty: true
            }),
            Decision::Preview
        );
        let d = dry_run_data("post");
        assert_eq!(d["validation"], "passed");
        assert_ne!(d["validation"], "would_succeed");
    }

    #[test]
    fn both_flags_is_exit_2() {
        assert!(matches!(
            decide(&ApplyInput {
                apply: true,
                dry_run: true,
                no_interactive: false,
                stdin_is_tty: true
            }),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn tty_without_flags_prompts() {
        assert_eq!(
            decide(&ApplyInput {
                apply: false,
                dry_run: false,
                no_interactive: false,
                stdin_is_tty: true
            }),
            Decision::Prompt
        );
    }

    #[test]
    fn no_interactive_or_pipe_without_flags_is_exit_2() {
        for (ni, tty) in [(true, true), (false, false), (true, false)] {
            assert!(matches!(
                decide(&ApplyInput {
                    apply: false,
                    dry_run: false,
                    no_interactive: ni,
                    stdin_is_tty: tty
                }),
                Decision::Deny(_)
            ));
        }
    }
}
