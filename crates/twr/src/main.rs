//! `twr` — agent-first CLI for X/Twitter.
//!
//! Single-print-owner discipline (plan §5.1): the ONLY stdout print of the
//! final envelope goes through `twr_core::emit`. Everything else is stderr.

use clap::{Parser, Subcommand};
use twr_core::{emit, Envelope, Meta, OutputFormat, OutputOptions};

#[derive(Parser)]
#[command(name = "twr", version, about = "Agent-first CLI for X/Twitter")]
struct Cli {
    /// Machine output as JSON.
    #[arg(long, global = true)]
    json: bool,
    /// Machine output as YAML (auto when piped without a flag).
    #[arg(long, global = true)]
    yaml: bool,
    /// Strip heavy fields (profile images, media dims, expanded urls).
    #[arg(long, short = 'c', global = true)]
    compact: bool,
    /// Project dotted-path fields, e.g. --fields id,text,author.screen_name.
    #[arg(long, global = true)]
    fields: Option<String>,
    /// Trace id (auto-generated when omitted).
    #[arg(long, global = true)]
    trace_id: Option<String>,
    /// Verbose diagnostics to stderr only (never stdout).
    #[arg(long, short = 'v', global = true)]
    verbose: bool,
    /// Request timeout override (seconds).
    #[arg(long, global = true)]
    timeout: Option<u64>,
    /// Max retries override.
    #[arg(long, global = true)]
    max_retries: Option<u32>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Auth gate — always call this first. See PLAN.md §5.4.
    Status,
    /// Print the envelope schema for every command's `type`. See PLAN.md §5.4.
    Schema,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let format = OutputFormat::resolve(cli.json, cli.yaml, atty_stdout());
    let mut opts = OutputOptions::new(cli.trace_id);
    opts.format = format;
    opts.compact = cli.compact;
    opts.fields = cli
        .fields
        .as_deref()
        .map(twr_core::parse_fields)
        .unwrap_or_default();
    opts.verbose = cli.verbose;
    opts.timeout_secs = cli.timeout;
    opts.max_retries = cli.max_retries;

    if opts.verbose {
        eprintln!("twr trace_id={} command starting", opts.trace_id);
    }

    let (kind, mut data) = match cli.command {
        Command::Status => (
            "status",
            serde_json::json!({
                "authenticated": false,
                "note": "twr-auth is not implemented yet — see PLAN.md P0/P1"
            }),
        ),
        Command::Schema => (
            "schema",
            serde_json::json!({
                "note": "Full JSON Schema per envelope type lands in P1 — see PLAN.md §5.4"
            }),
        ),
    };

    if opts.compact {
        data = twr_core::apply_compact(data);
    }
    if !opts.fields.is_empty() {
        data = twr_core::apply_fields(&data, &opts.fields);
    }

    let mut meta = Meta::new(opts.trace_id.clone());
    meta.command = Some(kind.to_string());
    let envelope = Envelope::ok(kind, data).with_meta(meta);

    // Single-print-owner: exactly one stdout print for the final envelope.
    match opts.format {
        OutputFormat::Json => emit(&envelope),
        OutputFormat::Yaml => emit_yaml(&envelope)?,
    }
    Ok(())
}

/// YAML sibling of `twr_core::emit` — the only other allowed stdout print.
/// (Two arms of one match = one owner: this and `emit` are the exclusive
/// envelope printers; grep `println!` in this file to audit.)
fn emit_yaml<T: serde::Serialize>(envelope: &twr_core::Envelope<T>) -> anyhow::Result<()> {
    let value = serde_json::to_value(envelope)?;
    println!("{}", serde_yaml::to_string(&value)?);
    Ok(())
}

fn atty_stdout() -> bool {
    // No atty dep: default to JSON unless --yaml/--json disambiguate.
    // (Auto-YAML-when-piped is resolved in OutputFormat::resolve by callers
    // that know the real TTY state; the binary defaults TTY=true here.)
    true
}
