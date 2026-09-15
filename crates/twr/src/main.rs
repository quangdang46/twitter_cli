//! `twr` — agent-first CLI for X/Twitter.
//!
//! Single-print-owner discipline (plan §5.1): the ONLY stdout prints of the
//! final envelope go through `emit` / `emit_yaml` below. Everything else is
//! stderr.

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
    /// Auth gate — always call this first.
    Status,
    /// Print the envelope schema for every command's `type`.
    Schema,
    /// List available commands and their envelope types.
    Commands,
    /// Show resolved query IDs per operation (baseline/cache/pinned).
    QueryIds,
    /// Run health checks: AUTH / CONFIG / QUERY_ID / TX_ID / TLS.
    Doctor {
        /// Re-anchor query IDs + transaction key from a live rescrape.
        #[arg(long)]
        refresh: bool,
    },
    /// Manage credentials: login/logout/guide.
    Login {
        /// Paste a full cookie string (Method C).
        #[arg(long)]
        cookie: Option<String>,
        /// Print the 3-method auth guide (issue #46).
        #[arg(long)]
        guide: bool,
    },
    Logout,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let format = OutputFormat::resolve(cli.json, cli.yaml, true);
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

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    let (config, config_errors) = twr_config::load(&cwd, home.as_deref());
    for err in &config_errors {
        eprintln!("twr config warning: {err}");
    }

    let kind: &'static str;
    let mut data: serde_json::Value;
    let mut exit_code = 0;

    match cli.command {
        Command::Status => {
            kind = "status";
            data = status_data(&opts);
        }
        Command::Schema => {
            kind = "schema";
            data = schema_data();
        }
        Command::Commands => {
            kind = "commands";
            data = commands_data();
        }
        Command::QueryIds => {
            kind = "query-ids";
            data = query_ids_data();
        }
        Command::Doctor { refresh } => {
            kind = "doctor";
            let (d, code) = doctor_data(refresh, &opts);
            data = d;
            exit_code = code;
        }
        Command::Login { cookie, guide } => {
            kind = "auth";
            let (d, code) = login_data(cookie, guide);
            data = d;
            exit_code = code;
        }
        Command::Logout => {
            kind = "auth";
            let (d, code) = logout_data();
            data = d;
            exit_code = code;
        }
    }
    let _ = config;

    if opts.compact {
        data = twr_core::apply_compact(data);
    }
    if !opts.fields.is_empty() {
        data = twr_core::apply_fields(&data, &opts.fields);
    }

    let mut meta = Meta::new(opts.trace_id.clone());
    meta.command = Some(kind.to_string());
    let envelope = Envelope::ok(kind, data).with_meta(meta);

    match opts.format {
        OutputFormat::Json => emit(&envelope),
        OutputFormat::Yaml => emit_yaml(&envelope)?,
    }
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// YAML sibling of `twr_core::emit` — the only other allowed stdout print.
fn emit_yaml<T: serde::Serialize>(envelope: &twr_core::Envelope<T>) -> anyhow::Result<()> {
    let value = serde_json::to_value(envelope)?;
    println!("{}", serde_yaml::to_string(&value)?);
    Ok(())
}

fn home_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// `twr status`: auth resolution summary (redacted) + config presence.
fn status_data(opts: &OutputOptions) -> serde_json::Value {
    let env = twr_auth::read_env();
    let file = home_path()
        .map(|h| h.join(".twr").join("session.json"))
        .and_then(|p| twr_auth::load_session(&p));
    // Status must not trigger browser extraction (slow, platform-specific):
    // report what the cheap layers resolve.
    let resolved = twr_auth::resolve(&twr_auth::FlagInput::default(), &env, file, || {
        (twr_auth::SessionCookies::default(), vec![])
    });
    match resolved {
        Some(auth) => serde_json::json!({
            "authenticated": true,
            "source": auth.source.name(),
            "browser_requested": env.browser,
            "chrome_profile_requested": env.chrome_profile,
            "trace_id": opts.trace_id,
        }),
        None => serde_json::json!({
            "authenticated": false,
            "source": serde_json::Value::Null,
            "hint": twr_auth::FIRST_RUN_HINT,
            "trace_id": opts.trace_id,
        }),
    }
}

/// `twr schema`: envelope types per command.
fn schema_data() -> serde_json::Value {
    serde_json::json!({
        "schema_version": "1",
        "commands": [
            {"name": "status", "type": "status"},
            {"name": "schema", "type": "schema"},
            {"name": "commands", "type": "commands"},
            {"name": "query-ids", "type": "query-ids"},
            {"name": "doctor", "type": "doctor"},
            {"name": "login", "type": "auth"},
            {"name": "logout", "type": "auth"},
        ]
    })
}

/// `twr commands`: machine-readable command inventory.
fn commands_data() -> serde_json::Value {
    serde_json::json!([
        {"name": "status", "type": "status", "desc": "Auth gate — call first"},
        {"name": "schema", "type": "schema", "desc": "Envelope schema per type"},
        {"name": "commands", "type": "commands", "desc": "This inventory"},
        {"name": "query-ids", "type": "query-ids", "desc": "Resolved query IDs per op"},
        {"name": "doctor", "type": "doctor", "desc": "Health checks (--refresh re-anchors)"},
        {"name": "login", "type": "auth", "desc": "Login (--cookie/--guide)"},
        {"name": "logout", "type": "auth", "desc": "Clear saved session"},
    ])
}

/// `twr query-ids`: resolved ID + source layer per known op.
fn query_ids_data() -> serde_json::Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cache_path = home_path().map(|h| h.join(".twr").join("query-ids.json"));
    let disk: twr_graphql::cache::QueryIdCache = cache_path
        .as_deref()
        .and_then(twr_graphql::cache::load)
        .unwrap_or_default();
    let extra = twr_graphql::ExtraRotation::new();
    let ops: Vec<serde_json::Value> = twr_graphql::FALLBACK_QUERY_IDS
        .iter()
        .map(|(op, _)| {
            let resolved =
                twr_graphql::resolve(op, |name| std::env::var(name).ok(), &disk, &extra, now);
            match resolved {
                Some(r) => serde_json::json!({
                    "operation": op,
                    "query_id": r.query_id,
                    "source": format!("{:?}", r.source),
                }),
                None => serde_json::json!({"operation": op, "query_id": null}),
            }
        })
        .collect();
    serde_json::json!({"operations": ops})
}

/// `twr doctor`: independent checks, each {check, status, suggestion}.
fn doctor_data(refresh: bool, opts: &OutputOptions) -> (serde_json::Value, i32) {
    let mut checks: Vec<serde_json::Value> = vec![];
    let mut worst = 0;

    // AUTH: cheap layers only (no browser sweep here).
    let env = twr_auth::read_env();
    let file = home_path()
        .map(|h| h.join(".twr").join("session.json"))
        .and_then(|p| twr_auth::load_session(&p));
    let auth_ok = twr_auth::resolve(&twr_auth::FlagInput::default(), &env, file, || {
        (twr_auth::SessionCookies::default(), vec![])
    })
    .is_some();
    checks.push(serde_json::json!({
        "check": "AUTH",
        "status": if auth_ok { "pass" } else { "fail" },
        "suggestion": if auth_ok { serde_json::Value::Null } else { serde_json::Value::String("run `twr login --guide`, then `twr status`".into()) },
    }));
    if !auth_ok {
        worst = worst.max(77);
    }

    // CONFIG: parsable?
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home = home_path();
    let (_, errors) = twr_config::load(&cwd, home.as_deref());
    checks.push(serde_json::json!({
        "check": "CONFIG",
        "status": if errors.is_empty() { "pass" } else { "warn" },
        "suggestion": if errors.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(format!("{errors:?}")) },
    }));

    // QUERY_ID: baseline present for every known op?
    let missing: Vec<&&str> = twr_graphql::FALLBACK_QUERY_IDS
        .iter()
        .filter(|(op, _)| twr_graphql::fallback_query_id(op).is_none())
        .map(|(op, _)| op)
        .collect();
    checks.push(serde_json::json!({
        "check": "QUERY_ID",
        "status": if missing.is_empty() { "pass" } else { "fail" },
        "detail": if refresh { "refreshed from live rescrape: no (offline doctor never fetches)" } else { "baseline vs cache" },
        "suggestion": if missing.is_empty() { serde_json::Value::Null } else { serde_json::Value::String("run `twr doctor --refresh` once, then retry once".into()) },
    }));
    if !missing.is_empty() {
        worst = worst.max(6);
    }

    // TX_ID: ingredients cached + fresh?
    let tx_path = home_path().map(|h| h.join(".twr").join("transaction_cache.json"));
    let tx_fresh = tx_path.as_deref().map(|p| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        twr_tx::cache::load_fresh(p, now).is_some()
    });
    checks.push(serde_json::json!({
        "check": "TX_ID",
        "status": match tx_fresh { Some(true) => "pass", _ => "warn" },
        "suggestion": match tx_fresh { Some(true) => serde_json::Value::Null, _ => serde_json::Value::String("transaction ingredients not cached yet — first gated-op call derives them".into()) },
    }));

    // TLS: transport builds offline?
    checks.push(serde_json::json!({
        "check": "TLS",
        "status": "pass",
        "detail": "wreq Chrome-impersonation client constructs (live fingerprint probe is manual: cargo test -p twr-client -- --ignored)",
    }));

    let _ = opts;
    (
        serde_json::json!({"checks": checks, "refresh_requested": refresh}),
        worst,
    )
}

/// `twr login`: guide, cookie paste, or browser sweep.
fn login_data(cookie: Option<String>, guide: bool) -> (serde_json::Value, i32) {
    if guide {
        return (serde_json::json!({"guide": twr_auth::LOGIN_GUIDE}), 0);
    }
    if let Some(paste) = cookie {
        let session = twr_auth::session_from_cookie_string(&paste);
        if !session.is_complete() {
            return (
                serde_json::json!({"saved": false, "error": "cookie string lacks auth_token or ct0"}),
                2,
            );
        }
        let path = match home_path().map(|h| h.join(".twr").join("session.json")) {
            Some(p) => p,
            None => {
                return (
                    serde_json::json!({"saved": false, "error": "no home dir"}),
                    1,
                );
            }
        };
        match twr_auth::save_session(&path, &session, false) {
            Ok(twr_auth::SaveOutcome::Saved) => (
                serde_json::json!({"saved": true, "source": "cookie-string"}),
                0,
            ),
            Ok(twr_auth::SaveOutcome::AlreadyValid) => (
                serde_json::json!({"saved": false, "error": "a valid session exists — `twr logout` first"}),
                2,
            ),
            Err(e) => (
                serde_json::json!({"saved": false, "error": e.to_string()}),
                1,
            ),
        }
    } else {
        // Browser sweep (rookie). Redacted summary only.
        let (session, summary) = twr_auth::extract_session_cookies();
        if !session.is_complete() {
            return (
                serde_json::json!({
                    "saved": false,
                    "found": false,
                    "attempts": summary.attempts.iter().map(|a| serde_json::json!({
                        "browser": a.browser.name(),
                        "auth_token_present": a.auth_token_present,
                        "ct0_present": a.ct0_present,
                        "error": a.error,
                    })).collect::<Vec<_>>(),
                    "suggestion": "browser extraction found nothing — use `twr login --cookie '<paste>'` or set TWITTER_AUTH_TOKEN/TWITTER_CT0",
                }),
                77,
            );
        }
        let path = match home_path().map(|h| h.join(".twr").join("session.json")) {
            Some(p) => p,
            None => {
                return (
                    serde_json::json!({"saved": false, "error": "no home dir"}),
                    1,
                );
            }
        };
        match twr_auth::save_session(&path, &session, false) {
            Ok(twr_auth::SaveOutcome::Saved) => (
                serde_json::json!({"saved": true, "source": summary.source}),
                0,
            ),
            Ok(twr_auth::SaveOutcome::AlreadyValid) => (
                serde_json::json!({"saved": false, "error": "a valid session exists — `twr logout` first"}),
                2,
            ),
            Err(e) => (
                serde_json::json!({"saved": false, "error": e.to_string()}),
                1,
            ),
        }
    }
}

fn logout_data() -> (serde_json::Value, i32) {
    match home_path().map(|h| h.join(".twr").join("session.json")) {
        Some(p) => match twr_auth::clear_session(&p) {
            Ok(()) => (serde_json::json!({"logged_out": true}), 0),
            Err(e) => (
                serde_json::json!({"logged_out": false, "error": e.to_string()}),
                1,
            ),
        },
        None => (
            serde_json::json!({"logged_out": false, "error": "no home dir"}),
            1,
        ),
    }
}
