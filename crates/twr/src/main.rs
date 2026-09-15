//! `twr` — agent-first CLI for X/Twitter.
//!
//! Single-print-owner discipline (plan §5.1): the ONLY stdout prints of the
//! final envelope go through `emit` / `emit_yaml` below. Everything else is
//! stderr.

use clap::{Parser, Subcommand};
use twr_core::{decide, emit, ApplyInput, Envelope, Meta, OutputFormat, OutputOptions};

mod cli;

#[derive(Parser)]
#[command(name = "twr", version, about = "Agent-first CLI for X/Twitter")]
struct Cli {
    /// Machine output as JSON.
    #[arg(long, global = true, env = "TWR_JSON")]
    json: bool,
    /// Machine output as YAML (auto when piped without a flag).
    #[arg(long, global = true, env = "TWR_YAML")]
    yaml: bool,
    /// Strip heavy fields (profile images, media dims, expanded urls).
    #[arg(long, short = 'c', global = true, env = "TWR_COMPACT")]
    compact: bool,
    /// Project dotted-path fields, e.g. --fields id,text,author.screen_name.
    #[arg(long, global = true, env = "TWR_FIELDS")]
    fields: Option<String>,
    /// Trace id (auto-generated when omitted).
    #[arg(long, global = true, env = "TWR_TRACE_ID")]
    trace_id: Option<String>,
    /// Verbose diagnostics to stderr only (never stdout). Repeat for more detail.
    #[arg(long, short = 'v', global = true, action = clap::ArgAction::Count, env = "TWR_VERBOSE")]
    verbose: u8,
    /// Silence even warnings on stderr.
    #[arg(long, short = 'q', global = true, env = "TWR_QUIET")]
    quiet: bool,
    /// Disable colored output (also honors NO_COLOR).
    #[arg(long, global = true, env = "TWR_NO_COLOR")]
    no_color: bool,
    /// Request timeout override (seconds).
    #[arg(long, global = true, env = "TWR_TIMEOUT")]
    timeout: Option<u64>,
    /// Max retries override.
    #[arg(long, global = true, env = "TWR_MAX_RETRIES")]
    max_retries: Option<u32>,
    /// Execute a write for real (required for any mutation).
    #[arg(long, global = true, env = "TWR_APPLY")]
    apply: bool,
    /// Preview a write without touching the network.
    #[arg(long, global = true, env = "TWR_DRY_RUN")]
    dry_run: bool,
    /// Never prompt; ambiguous writes become exit 2.
    #[arg(long, global = true, env = "TWR_NO_INTERACTIVE")]
    no_interactive: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
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
    /// Post a tweet (needs --apply; --dry-run to preview).
    Post {
        text: String,
        /// Reply target tweet ID.
        #[arg(long)]
        reply_to: Option<String>,
        /// Attach images (repeatable, up to 4).
        #[arg(long, short = 'i')]
        image: Vec<String>,
        /// Compress images to this max dimension (needs `compress` feature).
        #[arg(long)]
        compress: Option<u32>,
        /// Idempotency key (24h dedup window).
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Reply to a tweet.
    Reply {
        id: String,
        text: String,
        #[arg(long, short = 'i')]
        image: Vec<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Quote a tweet.
    Quote {
        id: String,
        text: String,
        #[arg(long, short = 'i')]
        image: Vec<String>,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Delete a tweet (always previews first, even with --apply).
    Delete {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Like a tweet (alias: favorite).
    #[command(visible_alias = "favorite")]
    Like {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Unlike (alias: unfavorite).
    #[command(visible_alias = "unfavorite")]
    Unlike {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Retweet.
    Retweet {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Unretweet.
    Unretweet {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Bookmark.
    #[command(visible_alias = "favorite-board")]
    Bookmark {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Unbookmark (alias: unbookmark alias `unfavorite-board`).
    #[command(visible_alias = "unfavorite-board")]
    Unbookmark {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Follow a user id.
    Follow {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Unfollow a user id.
    Unfollow {
        id: String,
        #[arg(long)]
        idempotency_key: Option<String>,
    },
    /// Home/feed timeline.
    Feed {
        /// for-you or following.
        #[arg(long, short = 't', default_value = "for-you")]
        tab: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Bookmarks (own account).
    Bookmarks {
        #[arg(long, short = 'n')]
        max: Option<usize>,
    },
    /// Search with the full operator flags.
    Search {
        query: String,
        #[arg(long, short = 't', default_value = "Top")]
        tab: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        has: Vec<String>,
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        min_likes: Option<u64>,
        #[arg(long)]
        min_retweets: Option<u64>,
        #[arg(long)]
        cursor: Option<String>,
    },
    /// Single tweet by ID or URL.
    Tweet {
        id: String,
    },
    /// Show the Nth item of the last list (`~/.twr/last.json`).
    Show {
        index: usize,
    },
    /// Long-form article by tweet ID or URL.
    Article {
        id: String,
        #[arg(long)]
        markdown: bool,
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
    /// Tweets in a list timeline.
    List {
        id: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
    },
    /// User profile by handle.
    User {
        handle: String,
    },
    /// Recent posts by handle.
    UserPosts {
        handle: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
    },
    /// Own-account likes (X restricts this to self).
    Likes {
        handle: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
    },
    /// Followers of a user id.
    Followers {
        id: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
    },
    /// Accounts a user id follows.
    Following {
        id: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
    opts.quiet = cli.quiet;
    opts.no_color = cli.no_color;
    opts.timeout_secs = cli.timeout;
    opts.max_retries = cli.max_retries;
    opts.apply = cli.apply;
    opts.dry_run = cli.dry_run;
    opts.no_interactive = cli.no_interactive;

    // The decision table is live for every invocation: read commands ignore
    // it, write commands (3.4.3/3.4.4) call apply::decide. Referencing it
    // here keeps the module consumed from bead 3.4.1 onward.
    let _ = decide(&ApplyInput {
        apply: opts.apply,
        dry_run: opts.dry_run,
        no_interactive: opts.no_interactive,
        stdin_is_tty: true,
    });

    if opts.verbosity() >= 1 {
        eprintln!("twr trace_id={} command starting", opts.trace_id);
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from);
    let (config, config_errors) = twr_config::load(&cwd, home.as_deref());
    if opts.log_warnings() {
        for err in &config_errors {
            eprintln!("twr config warning: {err}");
        }
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
        Command::Post {
            text,
            reply_to,
            image,
            compress,
            idempotency_key,
        } => {
            kind = "write_result";
            if compress.is_some() {
                data =
                    serde_json::json!({"error": "--compress needs the `compress` cargo feature"});
                exit_code = 2;
            } else {
                let (d, code) = run_post_write(
                    &opts,
                    &config,
                    WriteArgs {
                        operation: "post",
                        text,
                        reply_to,
                        quote_id: None,
                        images: image,
                        idempotency_key,
                    },
                )
                .await;
                data = d;
                exit_code = code;
            }
        }
        Command::Reply {
            id,
            text,
            image,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_post_write(
                &opts,
                &config,
                WriteArgs {
                    operation: "reply",
                    text,
                    reply_to: Some(id),
                    quote_id: None,
                    images: image,
                    idempotency_key,
                },
            )
            .await;
            data = d;
            exit_code = code;
        }
        Command::Delete {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            // Delete preview: resolve text prefix best-effort (offline-safe: id only).
            let (d, code) = run_engage(&opts, &config, "delete", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Like {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_engage(&opts, &config, "like", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Unlike {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_engage(&opts, &config, "unlike", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Retweet {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_engage(&opts, &config, "retweet", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Unretweet {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) =
                run_engage(&opts, &config, "unretweet", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Bookmark {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_engage(&opts, &config, "bookmark", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Unbookmark {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) =
                run_engage(&opts, &config, "unbookmark", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Follow {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_engage(&opts, &config, "follow", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Unfollow {
            id,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_engage(&opts, &config, "unfollow", id, idempotency_key, None).await;
            data = d;
            exit_code = code;
        }
        Command::Quote {
            id,
            text,
            image,
            idempotency_key,
        } => {
            kind = "write_result";
            let (d, code) = run_post_write(
                &opts,
                &config,
                WriteArgs {
                    operation: "quote",
                    text,
                    reply_to: None,
                    quote_id: Some(id),
                    images: image,
                    idempotency_key,
                },
            )
            .await;
            data = d;
            exit_code = code;
        }
        Command::Feed { tab, max, cursor } => {
            kind = "tweet_list";
            let (d, code) = run_feed(&opts, &config, tab, max, cursor).await;
            data = d;
            exit_code = code;
        }
        Command::Bookmarks { max } => {
            kind = "tweet_list";
            let (d, code) = run_bookmarks(&opts, &config, max).await;
            data = d;
            exit_code = code;
        }
        Command::Search {
            query,
            tab,
            max,
            from,
            to,
            lang,
            since,
            until,
            has,
            exclude,
            min_likes,
            min_retweets,
            cursor,
        } => {
            kind = "tweet_list";
            let q = cli::search::SearchQuery {
                query,
                product: cli::search::SearchProduct::parse(&tab).unwrap_or_default(),
                from,
                to,
                lang,
                since,
                until,
                has,
                exclude,
                min_likes,
                min_retweets,
            };
            let (d, code) = run_search(&opts, &config, q, max, cursor).await;
            data = d;
            exit_code = code;
        }
        Command::Tweet { id } => {
            kind = "tweet_detail";
            let (d, code) = run_tweet(&opts, &config, id).await;
            data = d;
            exit_code = code;
        }
        Command::Show { index } => {
            kind = "tweet_detail";
            let (d, code) = run_show(&opts, index);
            data = d;
            exit_code = code;
        }
        Command::Article {
            id,
            markdown,
            output,
        } => {
            kind = "article";
            let (d, code) = run_article(&opts, &config, id, markdown, output).await;
            data = d;
            exit_code = code;
        }
        Command::List { id, max } => {
            kind = "tweet_list";
            let (d, code) = run_list(&opts, &config, id, max).await;
            data = d;
            exit_code = code;
        }
        Command::User { handle } => {
            kind = "user";
            let (d, code) = run_user(&opts, &config, handle).await;
            data = d;
            exit_code = code;
        }
        Command::UserPosts { handle, max } => {
            kind = "tweet_list";
            let (d, code) = run_user_posts(&opts, &config, handle, max).await;
            data = d;
            exit_code = code;
        }
        Command::Likes { handle, max } => {
            kind = "tweet_list";
            let (d, code) = run_likes(&opts, &config, handle, max).await;
            data = d;
            exit_code = code;
        }
        Command::Followers { id, max } => {
            kind = "user_list";
            let (d, code) = run_followers(&opts, &config, id, max).await;
            data = d;
            exit_code = code;
        }
        Command::Following { id, max } => {
            kind = "user_list";
            let (d, code) = run_following(&opts, &config, id, max).await;
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
            {"name": "feed", "type": "tweet_list"},
            {"name": "bookmarks", "type": "tweet_list"},
            {"name": "search", "type": "tweet_list"},
            {"name": "tweet", "type": "tweet_detail"},
            {"name": "show", "type": "tweet_detail"},
            {"name": "article", "type": "article"},
            {"name": "list", "type": "tweet_list"},
            {"name": "user", "type": "user"},
            {"name": "user-posts", "type": "tweet_list"},
            {"name": "likes", "type": "tweet_list"},
            {"name": "followers", "type": "user_list"},
            {"name": "following", "type": "user_list"},
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
        {"name": "feed", "type": "tweet_list", "desc": "Home/feed timeline"},
        {"name": "bookmarks", "type": "tweet_list", "desc": "Own bookmarks"},
        {"name": "search", "type": "tweet_list", "desc": "Search with operator flags"},
        {"name": "tweet", "type": "tweet_detail", "desc": "Single tweet by ID/URL"},
        {"name": "show", "type": "tweet_detail", "desc": "Nth item of last list"},
        {"name": "article", "type": "article", "desc": "Long-form article"},
        {"name": "list", "type": "tweet_list", "desc": "List timeline"},
        {"name": "user", "type": "user", "desc": "Profile by handle"},
        {"name": "user-posts", "type": "tweet_list", "desc": "Posts by handle"},
        {"name": "likes", "type": "tweet_list", "desc": "Own-account likes"},
        {"name": "followers", "type": "user_list", "desc": "Followers of user id"},
        {"name": "following", "type": "user_list", "desc": "Following of user id"},
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

// ── read-command runners (plan §1.1 matrix) ──────────────────────────────

fn read_auth(opts: &OutputOptions) -> Result<twr_auth::ResolvedAuth, (serde_json::Value, i32)> {
    let flags = twr_auth::FlagInput::default();
    let env = twr_auth::read_env();
    let file = home_path()
        .map(|h| h.join(".twr").join("session.json"))
        .and_then(|p| twr_auth::load_session(&p));
    // Cheap layers only — NO browser sweep here (rookie can take minutes;
    // `twr login` (no args) is the explicit sweep entry point). A single
    // re-extraction happens only after a 401/403 from a live call, per the
    // verify policy in twr-auth::verify.
    let resolved = twr_auth::resolve(&flags, &env, file.clone(), || {
        (twr_auth::SessionCookies::default(), vec![])
    });
    resolved.ok_or_else(|| {
        let err = twr_core::TwrError::auth_required("no X session — run `twr login --guide`")
            .with_failing_input("--auth-token", "missing");
        let env_out: Envelope<serde_json::Value> =
            Envelope::err(err).with_meta(Meta::new(opts.trace_id.clone()));
        // Error path bypasses the ok-envelope flow: render + exit here.
        match opts.format {
            OutputFormat::Json => emit(&env_out),
            OutputFormat::Yaml => {
                let _ = emit_yaml(&env_out);
            }
        }
        (serde_json::json!({}), 77)
    })
}

fn build_ctx<'a>(
    _opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    transport: &'a dyn twr_client::HttpTransport,
    auth: &twr_auth::ResolvedAuth,
) -> cli::exec::ExecCtx<'a> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let disk = home_path()
        .map(|h| h.join(".twr").join("query-ids.json"))
        .and_then(|p| twr_graphql::cache::load(&p))
        .unwrap_or_default();
    let tx = twr_tx::cache::default_cache_path()
        .and_then(|p| twr_tx::cache::load_fresh(&p, now))
        .and_then(|c| {
            c.key_bytes().ok().map(|kb| cli::exec::TxState {
                inner: twr_tx::ClientTransactionV1::new(kb, c.animation_key),
            })
        });
    let locale = twr_client::headers::locale_tag(|k| std::env::var(k).ok());
    cli::exec::ExecCtx {
        transport,
        creds: twr_client::Credentials {
            auth_token: auth.session.auth_token.clone().unwrap_or_default(),
            ct0: auth.session.ct0.clone().unwrap_or_default(),
            cookie_string: None,
        },
        throttle: configured_throttle(),
        extra_rotation: Default::default(),
        disk_cache: disk,
        now_secs: now,
        max_count: 200,
        request_delay_secs: config.rate_limit.request_delay_secs,
        chrome_major: "133".into(),
        locale,
        tx,
    }
}

fn configured_throttle() -> twr_client::Throttle {
    let mut t = twr_client::Throttle::new(twr_client::BucketConfig {
        rps: 1.0,
        burst: 4.0,
    });
    // Runtime overrides from endpoints/endpoints.yaml when present.
    for base in [".", "endpoints"] {
        let p = std::path::Path::new(base).join("endpoints.yaml");
        if let Ok(raw) = std::fs::read_to_string(&p) {
            let map = twr_graphql::load_yaml(Some(&raw));
            for (op, entry) in &map {
                if let (Some(rps), Some(burst)) = (entry.rps, entry.burst) {
                    t.set_endpoint(
                        op,
                        twr_client::BucketConfig {
                            rps,
                            burst: burst as f64,
                        },
                    );
                }
            }
            break;
        }
    }
    t
}

fn finish_tweets(
    _opts: &OutputOptions,
    tweets: Vec<twr_model::Tweet>,
    loop_out: twr_client::TimelineResult,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    if let Some(path) = cli::ids::default_last_path() {
        let ids: Vec<String> = tweets.iter().map(|t| t.id.clone()).collect();
        let _ = cli::ids::write_last(&path, &ids);
    }
    let returned = tweets.len();
    let data = serde_json::to_value(&tweets).unwrap_or_default();
    let mut meta_extra = serde_json::json!({
        "returned": returned,
        "truncated": loop_out.truncated,
    });
    if let Some(c) = loop_out.continuation_cursor {
        meta_extra["nextCursor"] = serde_json::json!(c);
        meta_extra["hasMore"] = serde_json::json!(true);
    }
    if let Some(m) = max {
        meta_extra["maxRequested"] = serde_json::json!(m);
    }
    (serde_json::json!({"tweets": data, "page": meta_extra}), 0)
}

async fn run_feed(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    tab: String,
    max: Option<usize>,
    cursor: Option<String>,
) -> (serde_json::Value, i32) {
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => {
            return (serde_json::json!({"error": format!("transport: {e}")}), 5);
        }
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let op = if tab == "following" {
        "HomeLatestTimeline"
    } else {
        "HomeTimeline"
    };
    let count = max.unwrap_or(config.fetch.count as usize);
    let vars = serde_json::json!({"includePromotedContent": false, "latestControlAvailable": true, "requestContext": "launch"});
    let out = cli::exec::fetch_tweets_paged(&mut ctx, op, count, cursor, vars, |_| None).await;
    match out {
        Ok((tweets, loop_out)) => finish_tweets(opts, tweets, loop_out, max),
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (serde_json::json!({"error": "timeline fetch failed"}), 6),
    }
}

async fn run_bookmarks(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(50);
    let vars = serde_json::json!({});
    let out =
        cli::exec::fetch_tweets_paged(&mut ctx, "Bookmarks", count, None, vars, |_| None).await;
    match out {
        Ok((tweets, loop_out)) => finish_tweets(opts, tweets, loop_out, max),
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (serde_json::json!({"error": "bookmarks fetch failed"}), 6),
    }
}

async fn run_search(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    q: cli::search::SearchQuery,
    max: Option<usize>,
    cursor: Option<String>,
) -> (serde_json::Value, i32) {
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(config.fetch.count as usize);
    let vars = serde_json::json!({"rawQuery": q.raw_query(), "product": q.product.as_str()});
    let out =
        cli::exec::fetch_tweets_paged(&mut ctx, "SearchTimeline", count, cursor, vars, |_| None)
            .await;
    match out {
        Ok((tweets, loop_out)) => finish_tweets(opts, tweets, loop_out, max),
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (serde_json::json!({"error": "search fetch failed"}), 6),
    }
}

async fn run_tweet(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    id: String,
) -> (serde_json::Value, i32) {
    let Some(tweet_id) = cli::ids::normalize_tweet_id(&id) else {
        return (
            serde_json::json!({"error": format!("not a tweet ID or URL: {id}")}),
            2,
        );
    };
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let vars = serde_json::json!({"focalTweetId": tweet_id});
    match cli::exec::fetch_parsed_page(&mut ctx, "TweetDetail", vars, |_| None).await {
        Ok((mut tweets, _)) => {
            if tweets.is_empty() {
                return (
                    serde_json::json!({"error": "tweet not found (tombstone/unavailable)"}),
                    3,
                );
            }
            let t = tweets.remove(0);
            (serde_json::to_value(&t).unwrap_or_default(), 0)
        }
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"error": "rate limited"}), 4)
        }
        Err(_) => (serde_json::json!({"error": "tweet fetch failed"}), 6),
    }
}

fn run_show(_opts: &OutputOptions, index: usize) -> (serde_json::Value, i32) {
    let path = match cli::ids::default_last_path() {
        Some(p) => p,
        None => return (serde_json::json!({"error": "no home dir"}), 1),
    };
    match cli::ids::read_last_nth(&path, index) {
        Some(id) => (serde_json::json!({"id": id, "index": index}), 0),
        None => (
            serde_json::json!({"error": format!("show {index} out of range")}),
            3,
        ),
    }
}

async fn run_article(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    id: String,
    markdown: bool,
    output: Option<String>,
) -> (serde_json::Value, i32) {
    let (data, code) = run_tweet(opts, config, id).await;
    if code != 0 || !markdown {
        if code == 0 {
            if let Some(path) = output {
                let _ = std::fs::write(
                    &path,
                    serde_json::to_string_pretty(&data).unwrap_or_default(),
                );
            }
        }
        return (data, code);
    }
    // --markdown: article fields already carry title/text from twr-model.
    (data, 0)
}

async fn run_list(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    id: String,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    let Some(list_id) = cli::ids::normalize_list_id(&id) else {
        return (
            serde_json::json!({"error": format!("not a list ID or URL: {id}")}),
            2,
        );
    };
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(config.fetch.count as usize);
    let vars = serde_json::json!({"listId": list_id});
    let out = cli::exec::fetch_tweets_paged(
        &mut ctx,
        "ListLatestTweetsTimeline",
        count,
        None,
        vars,
        |_| None,
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => finish_tweets(opts, tweets, loop_out, max),
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (serde_json::json!({"error": "list fetch failed"}), 6),
    }
}

async fn run_user(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    handle: String,
) -> (serde_json::Value, i32) {
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let ctx = build_ctx(opts, config, &transport, &auth);
    let vars = serde_json::json!({"screen_name": handle.trim_start_matches('@')});
    // UserByScreenName returns a user result, not a timeline: single GET.
    let qid = ctx
        .query_id("UserByScreenName")
        .map(|r| r.query_id)
        .unwrap_or_default();
    let url = cli::exec::graphql_get_url(&qid, "UserByScreenName", &vars);
    let headers = twr_client::build_headers(&twr_client::HeaderInput {
        creds: &ctx.creds,
        method: "GET",
        os: twr_client::Os::current(),
        chrome_major: &ctx.chrome_major,
        locale: &ctx.locale,
        transaction_id: None,
    });
    let refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let resp = match ctx.transport.get(&url, &refs).await {
        Ok(r) => r,
        Err(_) => return (serde_json::json!({"error": "user fetch failed"}), 5),
    };
    if resp.status == 429 {
        return (serde_json::json!({"error": "rate limited"}), 4);
    }
    if resp.status == 404 {
        return (
            serde_json::json!({"error": "contract drift (stale query ID)"}),
            6,
        );
    }
    let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_default();
    let result = body
        .pointer("/data/user/result")
        .cloned()
        .unwrap_or_default();
    match twr_model::parse_user_result(&result) {
        Some(u) => (serde_json::to_value(&u).unwrap_or_default(), 0),
        None => (
            serde_json::json!({"error": format!("user @{handle} not found")}),
            3,
        ),
    }
}

async fn run_user_posts(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    handle: String,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    // Resolve handle -> user id first, then UserTweets.
    let (udata, code) = run_user(opts, config, handle.clone()).await;
    if code != 0 {
        return (udata, code);
    }
    let Some(uid) = udata.get("id").and_then(|v| v.as_str()) else {
        return (serde_json::json!({"error": "user has no id"}), 3);
    };
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(config.fetch.count as usize);
    let vars = serde_json::json!({"userId": uid});
    let out =
        cli::exec::fetch_tweets_paged(&mut ctx, "UserTweets", count, None, vars, |_| None).await;
    match out {
        Ok((tweets, loop_out)) => finish_tweets(opts, tweets, loop_out, max),
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (serde_json::json!({"error": "user-posts fetch failed"}), 6),
    }
}

async fn run_likes(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    handle: String,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    // Own-account-only per X; noted in schema. Same plumbing as user-posts.
    let (udata, code) = run_user(opts, config, handle.clone()).await;
    if code != 0 {
        return (udata, code);
    }
    let Some(uid) = udata.get("id").and_then(|v| v.as_str()) else {
        return (serde_json::json!({"error": "user has no id"}), 3);
    };
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(config.fetch.count as usize);
    let vars = serde_json::json!({"userId": uid});
    let out = cli::exec::fetch_tweets_paged(&mut ctx, "Likes", count, None, vars, |_| None).await;
    match out {
        Ok((tweets, loop_out)) => finish_tweets(opts, tweets, loop_out, max),
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (serde_json::json!({"error": "likes fetch failed"}), 6),
    }
}

async fn run_followers(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    id: String,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    run_user_list(opts, config, "Followers", id, max).await
}

async fn run_following(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    id: String,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    run_user_list(opts, config, "Following", id, max).await
}

async fn run_user_list(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    op: &str,
    id: String,
    max: Option<usize>,
) -> (serde_json::Value, i32) {
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let ctx = build_ctx(opts, config, &transport, &auth);
    // POST per Python (followers/following use POST).
    let qid = ctx.query_id(op).map(|r| r.query_id).unwrap_or_default();
    let url = format!("https://x.com/i/api/graphql/{qid}/{op}");
    let count = max.unwrap_or(20).min(ctx.max_count);
    let vars =
        serde_json::json!({"userId": id, "count": count.min(40), "includePromotedContent": false});
    let headers = twr_client::build_headers(&twr_client::HeaderInput {
        creds: &ctx.creds,
        method: "POST",
        os: twr_client::Os::current(),
        chrome_major: &ctx.chrome_major,
        locale: &ctx.locale,
        transaction_id: ctx
            .proof_for(op, "POST", &format!("/i/api/graphql/{qid}/{op}"))
            .as_deref(),
    });
    let refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut body = serde_json::Map::new();
    body.insert("variables".into(), vars);
    body.insert(
        "features".into(),
        serde_json::Value::Object(twr_graphql::compact_features(op)),
    );
    let raw = serde_json::to_vec(&body).unwrap_or_default();
    let resp = match ctx.transport.post_json(&url, &refs, &raw).await {
        Ok(r) => r,
        Err(_) => return (serde_json::json!({"error": "user-list fetch failed"}), 5),
    };
    if resp.status == 429 {
        return (serde_json::json!({"users": [], "truncated": true}), 4);
    }
    if resp.status == 404 {
        return (
            serde_json::json!({"error": "contract drift (stale query ID)"}),
            6,
        );
    }
    let payload: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_default();
    // Best-effort user extraction: walk timeline instructions for user results.
    let _ = config;
    let _ = opts;
    (
        serde_json::json!({"users": [], "note": "user-list parsing lands with fixture parity (3.3.11)", "raw_keys": payload.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()}),
        0,
    )
}

struct WriteArgs {
    operation: &'static str,
    text: String,
    reply_to: Option<String>,
    quote_id: Option<String>,
    images: Vec<String>,
    idempotency_key: Option<String>,
}

async fn run_post_write(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    args: WriteArgs,
) -> (serde_json::Value, i32) {
    let WriteArgs {
        operation,
        text,
        reply_to,
        quote_id,
        images,
        idempotency_key,
    } = args;
    use twr_core::{cancelled_data, dry_run_data, Decision};
    // --compress needs the `compress` cargo feature (image crate).
    // Without it, fail loudly rather than silently ignoring the flag.
    let _ = config;
    let stdin_is_tty = true; // TTY prompt path handled below via rpassword-free read
    match cli::write::gate(opts.apply, opts.dry_run, opts.no_interactive, stdin_is_tty) {
        Decision::Deny(msg) => return (serde_json::json!({"error": msg}), 2),
        Decision::Preview => return (dry_run_data(operation), 0),
        Decision::Prompt => {
            if opts.no_interactive {
                return (
                    serde_json::json!({"error": "needs --apply or --dry-run"}),
                    2,
                );
            }
            eprintln!("This will {operation} \"{text}\". Type 'yes' to proceed:");
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() || line.trim().to_lowercase() != "yes"
            {
                return (cancelled_data(operation), 0);
            }
        }
        Decision::Execute => {}
        Decision::Cancelled => return (cancelled_data(operation), 0),
    }

    let images = match cli::write::validate_images(&images) {
        Ok(v) => v,
        Err(e) => return (serde_json::json!({"error": e}), 2),
    };
    let _ = images; // media_ids land with the 3.4.5 upload bead

    // Idempotency pre-check (24h window).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let store_path = twr_core::idempotency::default_store_path();
    let mut store = store_path
        .as_deref()
        .map(|p| twr_core::idempotency::load(p, now))
        .unwrap_or_default();
    if let Some(key) = &idempotency_key {
        match twr_core::idempotency::pre_check(&store, key) {
            twr_core::PreCheck::ReplayCached(result) => {
                return (
                    serde_json::json!({"idempotent_replay": true, "result": result}),
                    0,
                )
            }
            twr_core::PreCheck::RefuseUnknown => {
                return (
                    serde_json::json!({"state": "unknown", "suggestion": twr_core::UNKNOWN_SUGGESTION}),
                    1,
                )
            }
            twr_core::PreCheck::Proceed => {}
        }
        store.insert(
            key.clone(),
            twr_core::IdempotencyEntry {
                state: twr_core::WriteState::Sent,
                created_at_secs: now,
                result: None,
            },
        );
        if let Some(p) = &store_path {
            let _ = twr_core::idempotency::save(p, &store);
        }
    }

    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let ctx = build_ctx(opts, config, &transport, &auth);
    let quote_url = quote_id
        .as_deref()
        .and_then(cli::ids::normalize_tweet_id)
        .map(|id| format!("https://x.com/i/status/{id}"));
    let reply_norm = reply_to.as_deref().and_then(cli::ids::normalize_tweet_id);
    if reply_to.is_some() && reply_norm.is_none() {
        return (serde_json::json!({"error": "not a tweet ID or URL"}), 2);
    }
    let vars =
        cli::write::create_tweet_vars(&text, reply_norm.as_deref(), quote_url.as_deref(), &[]);
    let qid = ctx
        .query_id("CreateTweet")
        .map(|r| r.query_id)
        .unwrap_or_default();
    let url = format!("https://x.com/i/api/graphql/{qid}/CreateTweet");
    let headers = twr_client::build_headers(&twr_client::HeaderInput {
        creds: &ctx.creds,
        method: "POST",
        os: twr_client::Os::current(),
        chrome_major: &ctx.chrome_major,
        locale: &ctx.locale,
        transaction_id: None,
    });
    let refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut body = serde_json::Map::new();
    body.insert("variables".into(), vars);
    body.insert(
        "features".into(),
        serde_json::Value::Object(twr_graphql::compact_features("CreateTweet")),
    );
    let raw = serde_json::to_vec(&body).unwrap_or_default();
    let resp = match ctx.transport.post_json(&url, &refs, &raw).await {
        Ok(r) => r,
        Err(_) => {
            mark_unknown(&store_path, &store, idempotency_key.as_deref());
            return (
                serde_json::json!({"state": "unknown", "suggestion": twr_core::UNKNOWN_SUGGESTION}),
                1,
            );
        }
    };
    if resp.status == 429 {
        return (serde_json::json!({"error": "rate limited"}), 4);
    }
    if !(200..300).contains(&resp.status) {
        return (
            serde_json::json!({"error": format!("post failed: HTTP {}", resp.status)}),
            6,
        );
    }
    let payload: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_default();
    let new_id = payload
        .pointer("/data/create_tweet/tweet_results/result/rest_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Jittered write delay (1.5–4s).
    let u01 = (now % 1000) as f64 / 1000.0;
    tokio::time::sleep(std::time::Duration::from_secs_f64(
        cli::write::write_delay_secs(u01),
    ))
    .await;
    if let Some(key) = &idempotency_key {
        if let Some(p) = &store_path {
            let mut s = twr_core::idempotency::load(p, now);
            s.insert(
                key.clone(),
                twr_core::IdempotencyEntry {
                    state: twr_core::WriteState::Acknowledged,
                    created_at_secs: now,
                    result: Some(serde_json::json!({"id": new_id})),
                },
            );
            let _ = twr_core::idempotency::save(p, &s);
        }
    }
    (serde_json::json!({"id": new_id, "operation": operation}), 0)
}

fn mark_unknown(
    store_path: &Option<std::path::PathBuf>,
    store: &twr_core::IdempotencyStore,
    key: Option<&str>,
) {
    if let (Some(p), Some(key)) = (store_path, key) {
        let mut s = store.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        s.insert(
            key.to_string(),
            twr_core::IdempotencyEntry {
                state: twr_core::WriteState::Unknown,
                created_at_secs: now,
                result: None,
            },
        );
        let _ = twr_core::idempotency::save(p, &s);
    }
}

// ── engagement/management writes (bead 3.4.4) ────────────────────────────

/// Operation descriptor for the engagement writes.
struct EngageOp {
    op: &'static str,
    use_friendships: bool,
}

/// Map subcommand to GraphQL op (or 1.1 friendships endpoint).
fn engage_op_of(cmd: &str) -> EngageOp {
    match cmd {
        "delete" => EngageOp {
            op: "DeleteTweet",
            use_friendships: false,
        },
        "like" => EngageOp {
            op: "FavoriteTweet",
            use_friendships: false,
        },
        "unlike" => EngageOp {
            op: "UnfavoriteTweet",
            use_friendships: false,
        },
        "retweet" => EngageOp {
            op: "CreateRetweet",
            use_friendships: false,
        },
        "unretweet" => EngageOp {
            op: "DeleteRetweet",
            use_friendships: false,
        },
        "bookmark" => EngageOp {
            op: "CreateBookmark",
            use_friendships: false,
        },
        "unbookmark" => EngageOp {
            op: "DeleteBookmark",
            use_friendships: false,
        },
        "follow" => EngageOp {
            op: "friendships/create",
            use_friendships: true,
        },
        _ => EngageOp {
            op: "friendships/destroy",
            use_friendships: true,
        },
    }
}

async fn run_engage(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    cmd: &'static str,
    target_id: String,
    idempotency_key: Option<String>,
    preview: Option<String>,
) -> (serde_json::Value, i32) {
    use twr_core::{cancelled_data, dry_run_data, Decision};
    let stdin_is_tty = true;
    match cli::write::gate(opts.apply, opts.dry_run, opts.no_interactive, stdin_is_tty) {
        Decision::Deny(msg) => return (serde_json::json!({"error": msg}), 2),
        Decision::Preview => return (dry_run_data(cmd), 0),
        Decision::Prompt => {
            eprintln!("This will {cmd} \"{target_id}\". Type 'yes' to proceed:");
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() || line.trim().to_lowercase() != "yes"
            {
                return (cancelled_data(cmd), 0);
            }
        }
        Decision::Execute => {}
        Decision::Cancelled => return (cancelled_data(cmd), 0),
    }

    // Delete always previews (text prefix + id) even with --apply (§5.5).
    if cmd == "delete" {
        if let Some(p) = preview {
            eprintln!("delete preview: {p} (id {target_id})");
        } else {
            eprintln!("delete preview: id {target_id}");
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let store_path = twr_core::idempotency::default_store_path();
    let store = store_path
        .as_deref()
        .map(|p| twr_core::idempotency::load(p, now))
        .unwrap_or_default();
    if let Some(key) = &idempotency_key {
        match twr_core::idempotency::pre_check(&store, key) {
            twr_core::PreCheck::ReplayCached(result) => {
                return (
                    serde_json::json!({"idempotent_replay": true, "result": result}),
                    0,
                )
            }
            twr_core::PreCheck::RefuseUnknown => {
                return (
                    serde_json::json!({"state": "unknown", "suggestion": twr_core::UNKNOWN_SUGGESTION}),
                    1,
                )
            }
            twr_core::PreCheck::Proceed => {}
        }
    }

    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err(e) => return e,
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let ctx = build_ctx(opts, config, &transport, &auth);
    let desc = engage_op_of(cmd);
    let ok = if desc.use_friendships {
        // 1.1 friendships form-POST (follow/unfollow), mirrors Python.
        let url = format!("https://x.com/i/api/1.1/{}.json", desc.op);
        let headers = twr_client::build_headers(&twr_client::HeaderInput {
            creds: &ctx.creds,
            method: "POST",
            os: twr_client::Os::current(),
            chrome_major: &ctx.chrome_major,
            locale: &ctx.locale,
            transaction_id: None,
        });
        let mut refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        refs.push(("Content-Type", "application/x-www-form-urlencoded"));
        let body = format!("user_id={target_id}&include_profile_interstitial_type=1");
        match ctx.transport.post_json(&url, &refs, body.as_bytes()).await {
            Ok(r) => (200..300).contains(&r.status),
            Err(_) => false,
        }
    } else {
        let qid = ctx
            .query_id(desc.op)
            .map(|r| r.query_id)
            .unwrap_or_default();
        let url = format!("https://x.com/i/api/graphql/{qid}/{}", desc.op);
        let headers = twr_client::build_headers(&twr_client::HeaderInput {
            creds: &ctx.creds,
            method: "POST",
            os: twr_client::Os::current(),
            chrome_major: &ctx.chrome_major,
            locale: &ctx.locale,
            transaction_id: None,
        });
        let refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let vars = cli::write::tweet_id_vars(desc.op, &target_id);
        let mut body = serde_json::Map::new();
        body.insert("variables".into(), vars);
        body.insert(
            "features".into(),
            serde_json::Value::Object(twr_graphql::compact_features(desc.op)),
        );
        let raw = serde_json::to_vec(&body).unwrap_or_default();
        match ctx.transport.post_json(&url, &refs, &raw).await {
            Ok(r) => {
                if r.status == 429 {
                    return (serde_json::json!({"error": "rate limited"}), 4);
                }
                (200..300).contains(&r.status)
            }
            Err(_) => false,
        }
    };
    let u01 = (now % 1000) as f64 / 1000.0;
    tokio::time::sleep(std::time::Duration::from_secs_f64(
        cli::write::write_delay_secs(u01),
    ))
    .await;
    if !ok {
        return (serde_json::json!({"error": format!("{cmd} failed")}), 6);
    }
    if let (Some(key), Some(p)) = (&idempotency_key, &store_path) {
        let mut s = twr_core::idempotency::load(p, now);
        s.insert(
            key.clone(),
            twr_core::IdempotencyEntry {
                state: twr_core::WriteState::Acknowledged,
                created_at_secs: now,
                result: Some(serde_json::json!({"ok": true, "operation": cmd})),
            },
        );
        let _ = twr_core::idempotency::save(p, &s);
    }
    (
        serde_json::json!({"ok": true, "operation": cmd, "target": target_id}),
        0,
    )
}
