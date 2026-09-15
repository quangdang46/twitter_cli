//! Root output flags: `--json/--yaml/--compact/--fields/--trace-id/-v`,
//! `--timeout/--max-retries` overrides (plan §5.3).
//!
//! `--toon` is intentionally NOT here — per the bead, TOON is a P3 renderer
//! over the same envelope, not a P1 output mode.

use crate::policy::Policy;
use crate::timefmt::TimeMode;

/// Which machine format to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Explicit `--json`, or auto when piped (plan §5.1).
    #[default]
    Json,
    Yaml,
    /// `--toon`: same envelope, tabular renderer.
    Toon,
}

impl OutputFormat {
    /// Resolve from flags: `--json` wins over `--yaml` when both are passed;
    /// when neither is passed, piped stdout (non-TTY) auto-selects YAML to
    /// match the Python original, TTY defaults to JSON.
    pub fn resolve(json: bool, yaml: bool, stdout_is_tty: bool) -> Self {
        Self::resolve_full(json, yaml, false, stdout_is_tty)
    }

    /// Full trio: --toon wins over --json/--yaml when combined.
    pub fn resolve_full(json: bool, yaml: bool, toon: bool, stdout_is_tty: bool) -> Self {
        if toon {
            OutputFormat::Toon
        } else if json {
            OutputFormat::Json
        } else if yaml {
            OutputFormat::Yaml
        } else if stdout_is_tty {
            OutputFormat::Json
        } else {
            OutputFormat::Yaml
        }
    }
}

/// Resolved root output options, shared by every command.
#[derive(Debug, Clone, Default)]
pub struct OutputOptions {
    pub format: OutputFormat,
    pub compact: bool,
    pub fields: Vec<String>,
    pub trace_id: String,
    /// Counted verbosity (-v/-vv); 0 = normal.
    pub verbose: u8,
    /// Silence even warnings on stderr (wins over verbose).
    pub quiet: bool,
    /// Disable colored output (also honors NO_COLOR env).
    pub no_color: bool,
    pub timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    /// Execute a write for real (required for any mutation, plan §5.3).
    pub apply: bool,
    /// Preview a write without touching the network.
    pub dry_run: bool,
    /// Never prompt; ambiguous writes become exit 2.
    pub no_interactive: bool,
    /// Write policy tier (plan §5.3 --policy).
    pub policy: Policy,
    /// Human-table time display (issue #35; machine output always absolute).
    pub time_mode: TimeMode,
    /// Human table shows full text (no 120-char truncation).
    pub full_text: bool,
    /// Access tier cap (guest degraded-mode reads).
    pub tier: Option<String>,
}

impl OutputOptions {
    /// True unless silenced by --quiet.
    pub fn log_warnings(&self) -> bool {
        !self.quiet
    }

    /// Stderr detail level after quiet is applied.
    pub fn verbosity(&self) -> u8 {
        if self.quiet {
            0
        } else {
            self.verbose
        }
    }

    /// Effective no-color: flag OR NO_COLOR env.
    pub fn effective_no_color(&self) -> bool {
        self.no_color || std::env::var_os("NO_COLOR").is_some()
    }
}

impl OutputOptions {
    /// Build with an auto-generated trace id when none is supplied.
    pub fn new(trace_id: Option<String>) -> Self {
        Self {
            trace_id: trace_id.unwrap_or_else(new_trace_id),
            ..Default::default()
        }
    }
}

/// Generate a trace id (uuid v4). Dependency-free (a simple RNG-based hex —
/// uniqueness per invocation is what matters, not RFC compliance).
pub fn new_trace_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    // 128-bit mix of time + pid + a counter-ish spin; hex-formatted 32 chars.
    let mut x = nanos ^ (pid << 64) ^ 0x9e3779b97f4a7c15;
    let mut out = String::with_capacity(32);
    for _ in 0..32 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push(char::from_digit((x & 0xf) as u32, 16).unwrap());
    }
    format!(
        "{}-{}-{}-{}-{}",
        &out[0..8],
        &out[8..12],
        &out[12..16],
        &out[16..20],
        &out[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_resolution_matches_python_auto_yaml_when_piped() {
        assert_eq!(OutputFormat::resolve(true, true, true), OutputFormat::Json);
        assert_eq!(OutputFormat::resolve(false, true, true), OutputFormat::Yaml);
        assert_eq!(
            OutputFormat::resolve(false, false, true),
            OutputFormat::Json
        );
        assert_eq!(
            OutputFormat::resolve(false, false, false),
            OutputFormat::Yaml
        );
    }

    #[test]
    fn quiet_wins_over_verbose_and_no_color_honors_env() {
        let mut opts = OutputOptions::new(Some("t".into()));
        opts.verbose = 2;
        assert_eq!(opts.verbosity(), 2);
        assert!(opts.log_warnings());
        opts.quiet = true;
        assert_eq!(opts.verbosity(), 0);
        assert!(!opts.log_warnings());
        assert!(!opts.effective_no_color());
        opts.no_color = true;
        assert!(opts.effective_no_color());
    }

    #[test]
    fn trace_ids_are_unique_and_dashed() {
        let a = new_trace_id();
        let b = new_trace_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(a.chars().filter(|&c| c == '-').count(), 4);
        assert_eq!(OutputOptions::new(None).trace_id.len(), 36);
        assert_eq!(OutputOptions::new(Some("t".into())).trace_id, "t");
    }
}
