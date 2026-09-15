//! `twr` — agent-first CLI for X/Twitter.
//!
//! This binary is a scaffold. Command implementation follows the phase plan
//! in `PLAN.md` (§9): P0 spike -> P1 read MVP -> P2 writes -> P3 polish.
//! Only `status` and `schema` exist today, and both are stubs that prove out
//! the envelope contract in `twr-core` before any network code lands.

use clap::{Parser, Subcommand};
use twr_core::Envelope;

#[derive(Parser)]
#[command(name = "twr", version, about = "Agent-first CLI for X/Twitter")]
struct Cli {
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
    match cli.command {
        Command::Status => {
            let envelope = Envelope::ok(
                "status",
                serde_json::json!({
                    "authenticated": false,
                    "note": "twr-auth is not implemented yet — see PLAN.md P0/P1"
                }),
            );
            println!("{}", serde_json::to_string(&envelope)?);
        }
        Command::Schema => {
            let envelope = Envelope::ok(
                "schema",
                serde_json::json!({
                    "note": "Full JSON Schema per envelope type lands in P1 — see PLAN.md §5.4"
                }),
            );
            println!("{}", serde_json::to_string(&envelope)?);
        }
    }
    Ok(())
}
