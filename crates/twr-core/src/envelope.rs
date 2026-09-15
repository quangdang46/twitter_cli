//! The agent-facing output envelope. See `PLAN.md` §5.1.
//!
//! Invariant: stdout carries exactly one of these documents per invocation.
//! Everything else (logs, warnings, progress) goes to stderr.

use serde::Serialize;

use crate::error::TwrError;

/// Top-level envelope returned by every `twr` command in machine mode
/// (`--json` / `--yaml` / `--toon`).
#[derive(Debug, Serialize)]
#[serde(tag = "ok")]
pub enum Envelope<T: Serialize> {
    #[serde(rename = "true")]
    Ok {
        schema_version: &'static str,
        #[serde(rename = "type")]
        kind: &'static str,
        data: T,
    },
    #[serde(rename = "false")]
    Err {
        schema_version: &'static str,
        #[serde(rename = "type")]
        kind: &'static str,
        error: TwrError,
    },
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(kind: &'static str, data: T) -> Self {
        Envelope::Ok {
            schema_version: "1",
            kind,
            data,
        }
    }
}
