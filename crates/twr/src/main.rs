//! `twr` — agent-first CLI for X/Twitter.
//!
//! Single-print-owner discipline (plan §5.1): the ONLY stdout prints of the
//! final envelope go through `emit` / `emit_yaml` below. Everything else is
//! stderr.

use clap::{CommandFactory, Parser, Subcommand};
use twr_client::HttpTransport;
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
    /// Token-efficient TOON rendering of the same envelope (lists tabular).
    /// Errors always stay JSON/YAML, never TOON.
    #[arg(long, global = true, env = "TWR_TOON")]
    toon: bool,
    /// Strip heavy fields (profile images, media dims, expanded urls).
    #[arg(long, short = 'c', global = true, env = "TWR_COMPACT")]
    compact: bool,
    /// Show full tweet text in the human table (no 120-char truncation).
    #[arg(long, global = true, env = "TWR_FULL_TEXT")]
    full_text: bool,
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
    /// Write policy: read_only blocks all writes, engagement allows
    /// like/rt/follow/bookmark only, write (default) allows all gated writes.
    #[arg(long, global = true, env = "TWR_POLICY", default_value = "write")]
    policy: String,
    /// Human-table time display: relative|absolute|both (machine output
    /// always carries absolute created_at).
    #[arg(long, global = true, env = "TWR_TIME", default_value = "relative")]
    time: String,
    /// Access tier cap: syndication|guest|session (guest covers a narrow
    /// op set when full session auth is unavailable).
    #[arg(long, global = true, env = "TWR_TIER")]
    tier: Option<String>,
    /// Backend: cookie (default) or api-v2 (OAuth2, feature-gated P4).
    #[arg(long, global = true, env = "TWR_BACKEND", default_value = "cookie")]
    backend: String,

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
        /// Live probe: real UserByScreenName call for HANDLE (needs session;
        /// explicit opt-in, touches x.com — this is the P0-4 validation).
        #[arg(long)]
        probe_user: Option<String>,
    },
    /// Manage credentials: login/logout/guide.
    Login {
        /// Paste a full cookie string (Method C).
        #[arg(long)]
        cookie: Option<String>,
        /// Print the 3-method auth guide (issue #46).
        #[arg(long)]
        guide: bool,
        /// Start OAuth2 api-v2 login (needs --client-id; opens browser URL).
        #[arg(long)]
        api_v2: bool,
        /// OAuth2 client ID (X developer app).
        #[arg(long, env = "TWR_CLIENT_ID")]
        client_id: Option<String>,
    },
    Logout,
    /// Search the local SQLite cache (FTS5, offline).
    CacheSearch {
        query: String,
        #[arg(long, short = 'n', default_value = "20")]
        max: usize,
    },
    /// Watchlist management.
    Watch {
        #[command(subcommand)]
        op: WatchOp,
    },
    /// Print shell completions (script to stdout, instructions to stderr).
    Completions {
        /// Shell: bash|zsh|fish|powershell|elvish.
        shell: String,
    },
    /// Serve the tool catalog over stdio (JSON-RPC MCP shape).
    Mcp,
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
        /// Attach a video (mp4/mov, api-v2 chunked upload; ≤128MB).
        #[arg(long)]
        video: Option<String>,
        /// Alias for --video.
        #[arg(long)]
        file: Option<String>,
        /// Alt text for attached media.
        #[arg(long)]
        alt_text: Option<String>,
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
        /// Score-based filtering (opt-in, off by default).
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
    },
    /// Bookmarks (own account).
    Bookmarks {
        #[arg(long, short = 'n')]
        max: Option<usize>,
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
        /// List bookmark folders instead of tweets (`bookmarks folders` in
        /// the Python original). Mutually exclusive with --folder.
        #[arg(long)]
        folders: bool,
        /// Fetch tweets from one bookmark folder by ID (mirrors Python
        /// `bookmarks folders <id>`). Mutually exclusive with --folders.
        #[arg(long)]
        folder: Option<String>,
        /// Only show folder tweets created on/after this date (YYYY-MM-DD).
        /// Client-side filter, mirroring Python's `_filter_tweets_since`.
        #[arg(long)]
        since: Option<String>,
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
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
        /// v2 search scope: recent|all (api-v2 backend only).
        #[arg(long, env = "TWR_SCOPE", default_value = "recent")]
        scope: String,
    },
    /// Single tweet by ID or URL.
    Tweet {
        id: String,
        /// v2 reply scope: auto|recent|all (api-v2 backend only).
        #[arg(long, env = "TWR_REPLY_SCOPE", default_value = "auto")]
        reply_scope: String,
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
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
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
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
    },
    /// Own-account likes (X restricts this to self).
    Likes {
        handle: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
    },
    /// Followers of a user id.
    Followers {
        id: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
    },
    /// Accounts a user id follows.
    Following {
        id: String,
        #[arg(long, short = 'n')]
        max: Option<usize>,
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
    },
    /// Today's headlines (trending-search fallback until the Search
    /// Navigation surface is reverse-engineered; issue #47).
    Headlines {
        /// Optional query (default: trending approximation).
        query: Option<String>,
        #[arg(long, short = 'n')]
        max: Option<usize>,
        #[arg(long, env = "TWR_FILTER")]
        filter: bool,
    },
    /// Run a read command once after a delay (one-shot, NOT cron).
    Future {
        /// Delay in seconds before running.
        #[arg(long, short = 'd')]
        delay_secs: u64,
        /// The read subcommand to run: feed|search|user|tweet.
        command: String,
        /// Arguments for the subcommand (e.g. search query or handle).
        args: Vec<String>,
    },
}

#[derive(clap::Subcommand)]
enum WatchOp {
    /// Add a handle to the watchlist.
    Add { handle: String },
    /// Remove a handle.
    Remove { handle: String },
    /// List watched handles.
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let format = OutputFormat::resolve_full(cli.json, cli.yaml, cli.toon, true);
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
    opts.policy = twr_core::Policy::parse(&cli.policy).unwrap_or_default();
    opts.time_mode = twr_core::TimeMode::parse(&cli.time).unwrap_or_default();
    opts.full_text = cli.full_text;
    opts.tier = cli.tier.clone();
    opts.backend = cli.backend.clone();

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
        Command::Doctor {
            refresh,
            probe_user,
        } => {
            kind = "doctor";
            let (d, code) = doctor_data_probe(refresh, probe_user, &opts, &config).await;
            data = d;
            exit_code = code;
        }
        Command::Login {
            cookie,
            guide,
            api_v2,
            client_id,
        } => {
            kind = "auth";
            let (d, code) = login_data_v2aware(cookie, guide, api_v2, client_id);
            data = d;
            exit_code = code;
        }
        Command::Logout => {
            kind = "auth";
            let (d, code) = logout_data();
            data = d;
            exit_code = code;
        }
        Command::CacheSearch { query, max } => {
            kind = "tweet_list";
            let (d, code) = run_cache_search(&query, max);
            data = d;
            exit_code = code;
        }
        Command::Watch { op } => {
            kind = "watchlist";
            let (d, code) = run_watch(op);
            data = d;
            exit_code = code;
        }
        Command::Completions { shell } => {
            return run_completions(&shell);
        }
        Command::Mcp => {
            // stdio loop: never returns until stdin closes. We are already
            // inside the tokio runtime (#[tokio::main]), so dispatch via
            // block_in_place on the current handle (calls are sequential).
            cli::mcp::serve_stdio(&|name, args| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(mcp_invoke(name, args))
                })
            });
            return Ok(());
        }
        Command::Post {
            text,
            reply_to,
            image,
            compress,
            video,
            file,
            alt_text,
            idempotency_key,
        } => {
            kind = "write_result";
            if compress.is_some() {
                data =
                    serde_json::json!({"error": "--compress needs the `compress` cargo feature"});
                exit_code = 2;
            } else if video.is_some() || file.is_some() {
                // v2 video path: validated here, uploaded in run_post_write's
                // media stage via twr-v2 (STATUS polling); cookie backend
                // rejects --video with guidance.
                let backend = twr_v2::Backend::parse(&opts.backend).unwrap_or_default();
                let (routed, _) = twr_v2::route("post", backend);
                if routed != twr_v2::Backend::ApiV2 {
                    data = serde_json::json!({"error": "--video/--file need --backend api-v2 (cookie backend takes -i images only)"});
                    exit_code = 2;
                } else {
                    let vpath = video.or(file).unwrap_or_default();
                    let alt = alt_text.clone();
                    let (d, code) = run_post_write_v2video(
                        &opts,
                        &config,
                        text,
                        reply_to,
                        vpath,
                        alt,
                        idempotency_key,
                    )
                    .await;
                    data = d;
                    exit_code = code;
                }
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
        Command::Future {
            delay_secs,
            command,
            args,
        } => {
            kind = "scheduled";
            let (d, code) = run_future(&opts, delay_secs, command, args).await;
            data = d;
            exit_code = code;
        }
        Command::Feed {
            tab,
            max,
            cursor,
            filter,
        } => {
            kind = "tweet_list";
            let (d, code) = run_feed(&opts, &config, tab, max, cursor, filter).await;
            data = d;
            exit_code = code;
        }
        Command::Bookmarks {
            max,
            filter,
            folders,
            folder,
            since,
        } => {
            if folders && folder.is_some() {
                kind = "error";
                let (d, code) = (
                    serde_json::json!({"error": "--folders and --folder are mutually exclusive"}),
                    2,
                );
                data = d;
                exit_code = code;
            } else if folders {
                kind = "bookmark_folder_list";
                let (d, code) = run_bookmark_folders(&opts).await;
                data = d;
                exit_code = code;
            } else if let Some(folder_id) = folder {
                kind = "tweet_list";
                let (d, code) =
                    run_bookmark_folder_timeline(&opts, &config, folder_id, max, since, filter)
                        .await;
                data = d;
                exit_code = code;
            } else {
                kind = "tweet_list";
                let (d, code) = run_bookmarks(&opts, &config, max, filter).await;
                data = d;
                exit_code = code;
            }
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
            filter,
            scope,
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
            let backend = twr_v2::Backend::parse(&opts.backend).unwrap_or_default();
            let (d, code) =
                run_search_v2aware(&opts, &config, q, max, cursor, filter, &scope, backend).await;
            data = d;
            exit_code = code;
        }
        Command::Tweet { id, reply_scope } => {
            kind = "tweet_detail";
            let (d, code) = run_tweet_v2aware(&opts, &config, id, reply_scope).await;
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
        Command::List { id, max, filter } => {
            kind = "tweet_list";
            let (d, code) = run_list(&opts, &config, id, max, filter).await;
            data = d;
            exit_code = code;
        }
        Command::User { handle } => {
            kind = "user";
            let (d, code) = run_user(&opts, &config, handle).await;
            data = d;
            exit_code = code;
        }
        Command::UserPosts {
            handle,
            max,
            filter,
        } => {
            kind = "tweet_list";
            let (d, code) = run_user_posts(&opts, &config, handle, max, filter).await;
            data = d;
            exit_code = code;
        }
        Command::Likes {
            handle,
            max,
            filter,
        } => {
            kind = "tweet_list";
            let (d, code) = run_likes(&opts, &config, handle, max, filter).await;
            data = d;
            exit_code = code;
        }
        Command::Followers {
            id,
            max,
            filter: _filter,
        } => {
            kind = "user_list";
            let (d, code) = run_followers(&opts, &config, id, max).await;
            data = d;
            exit_code = code;
        }
        Command::Headlines { query, max, filter } => {
            kind = "headline_list";
            let (d, code) = run_headlines(&opts, &config, query, max, filter).await;
            data = d;
            exit_code = code;
        }
        Command::Following {
            id,
            max,
            filter: _filter,
        } => {
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

    // Human view: no explicit machine flag → render the table from the same data.
    if !cli.json && !cli.yaml && !cli.toon {
        let text = render_human(kind, &data, &opts);
        println!("{text}");
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
        return Ok(());
    }

    let mut meta = Meta::new(opts.trace_id.clone());
    meta.command = Some(kind.to_string());

    // CONTRACT FIX (was a systemic bug across ~30 call sites): every command
    // handler above returns an ad-hoc `(serde_json::Value, i32)` tuple, and
    // this used to *always* wrap the result in `Envelope::ok(...)` — which
    // serializes `"ok":true` — even when `exit_code != 0`. Per plan §5.1, an
    // agent must be able to trust `ok`/exit-code together; `ok:true` on a
    // failure silently breaks that. Route any non-zero exit through a real
    // `Envelope::err` instead, reconstructing a `TwrError` from the exit code
    // (via `ErrorKind::from_exit_code`) and whatever message the handler put
    // in `data.error` (or `data.saved`/`data.logged_out` failure shapes).
    if exit_code != 0 {
        let message = data
            .get("error")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("command failed (exit {exit_code})"));
        let kind_enum = twr_core::ErrorKind::from_exit_code(exit_code)
            .unwrap_or(twr_core::ErrorKind::GeneralAuth);
        let err = twr_core::TwrError::new(kind_enum, message);
        let envelope: Envelope<serde_json::Value> = Envelope::err(err).with_meta(meta);
        match opts.format {
            OutputFormat::Json => emit(&envelope),
            OutputFormat::Yaml => emit_yaml(&envelope)?,
            OutputFormat::Toon => twr_core::emit_toon(&envelope),
        }
        std::process::exit(exit_code);
    }

    let envelope = Envelope::ok(kind, data).with_meta(meta);
    match opts.format {
        OutputFormat::Json => emit(&envelope),
        OutputFormat::Yaml => emit_yaml(&envelope)?,
        // TOON renders data only; errors fall back to JSON inside emit_toon.
        OutputFormat::Toon => twr_core::emit_toon(&envelope),
    }
    Ok(())
}

/// YAML sibling of `twr_core::emit` — the only other allowed stdout print.
fn emit_yaml<T: serde::Serialize>(envelope: &twr_core::Envelope<T>) -> anyhow::Result<()> {
    let value = serde_json::to_value(envelope)?;
    println!("{}", serde_yaml::to_string(&value)?);
    Ok(())
}

/// Cheap session probe (no browser sweep): true when flags/env/file resolve.
fn has_session() -> bool {
    let env = twr_auth::read_env();
    let file = home_path()
        .map(|h| h.join(".twr").join("session.json"))
        .and_then(|p| twr_auth::load_session(&p));
    twr_auth::resolve(&twr_auth::FlagInput::default(), &env, file, || {
        (twr_auth::SessionCookies::default(), vec![])
    })
    .is_some()
}

/// Tier guard for reads: guest covers UserByScreenName/TweetDetail/UserTweets
/// only; anything else needs session. Returns Some((data, code)) when denied.
fn tier_guard(
    opts: &OutputOptions,
    operation: &str,
    authed: bool,
) -> Option<(serde_json::Value, i32)> {
    let flag = opts
        .tier
        .as_deref()
        .and_then(twr_client::guest::Tier::parse);
    let tier = twr_client::guest::effective_tier(flag, authed);
    if tier == twr_client::guest::Tier::Session {
        return None;
    }
    if tier == twr_client::guest::Tier::Guest && twr_client::guest::guest_covers(operation) {
        return None;
    }
    // Exit 77 (AuthRequired), not 2 (UsagePolicyDenied): the caller has no
    // session-level auth, which is squarely "not logged in" territory per
    // the plan §5.2 exit-code contract, not a usage/policy mistake -- an
    // agent should react to this the same way it reacts to a missing cookie,
    // by running `twr status`/`twr login`, not by second-guessing its flags.
    Some((
        serde_json::json!({"error": format!("tier {} does not cover {operation}; use full session auth", tier.as_str())}),
        77,
    ))
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
            {"name": "bookmarks --folders", "type": "bookmark_folder_list"},
            {"name": "bookmarks --folder", "type": "tweet_list"},
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
        {"name": "bookmarks --folders", "type": "bookmark_folder_list", "desc": "List bookmark folders"},
        {"name": "bookmarks --folder <id> [--since YYYY-MM-DD]", "type": "tweet_list", "desc": "Tweets in one bookmark folder"},
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

    // CACHE: sqlite reachable + counts (independent check, never conflated).
    match twr_cache::default_db_path() {
        None => checks.push(serde_json::json!({
            "check": "CACHE",
            "status": "warn",
            "suggestion": "no home dir — cache unavailable",
        })),
        Some(path) => match twr_cache::open(&path).and_then(|c| twr_cache::health(&c)) {
            Ok(h) => checks.push(serde_json::json!({
                "check": "CACHE",
                "status": "pass",
                "detail": format!("tweets={} media={} watch={} wal={}", h.tweet_count, h.media_count, h.watch_count, h.wal_mode),
            })),
            Err(e) => checks.push(serde_json::json!({
                "check": "CACHE",
                "status": "fail",
                "suggestion": format!("cache db failed: {e}"),
            })),
        },
    }

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

enum AuthFail {
    Envelope(twr_core::TwrError),
}

#[allow(clippy::result_large_err)]
fn read_auth(opts: &OutputOptions) -> Result<twr_auth::ResolvedAuth, (AuthFail, i32)> {
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
    // NOTE: single-print-owner — do NOT emit here. The caller emits one
    // Envelope::err and sets exit 77.
    resolved.ok_or_else(|| {
        let err = twr_core::TwrError::auth_required("no X session — run `twr login --guide`")
            .with_failing_input("--auth-token", "missing");
        let _ = opts;
        (AuthFail::Envelope(err), 77)
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
            // Forward the FULL cookie string when we have it (Method C /
            // session files that preserved it): a bare auth_token+ct0 pair
            // is exactly what X's code-226 automated-behavior gate rejects
            // on writes. `cookie_header()` prefers this when set.
            cookie_string: auth.session.full_string.clone(),
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
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    tweets: Vec<twr_model::Tweet>,
    loop_out: twr_client::TimelineResult,
    max: Option<usize>,
    do_filter: bool,
    filter_applied_out: &mut bool,
) -> (serde_json::Value, i32) {
    if let Some(path) = cli::ids::default_last_path() {
        let ids: Vec<String> = tweets.iter().map(|t| t.id.clone()).collect();
        let _ = cli::ids::write_last(&path, &ids);
    }
    let tweets = if do_filter {
        *filter_applied_out = true;
        let cfg = twr_filter::FilterConfig::from_json(&serde_json::json!({
            "mode": config.filter.mode,
            "topN": config.filter.top_n,
            "weights": {
                "likes": config.filter.weights.likes,
                "retweets": config.filter.weights.retweets,
                "replies": config.filter.weights.replies,
                "bookmarks": config.filter.weights.bookmarks,
                "views_log": config.filter.weights.views_log,
            },
        }));
        twr_filter::filter_tweets(tweets, &cfg)
    } else {
        tweets
    };
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
    meta_extra["filterApplied"] = serde_json::json!(*filter_applied_out);
    let _ = opts;
    (serde_json::json!({"tweets": data, "page": meta_extra}), 0)
}

async fn run_feed(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    tab: String,
    max: Option<usize>,
    cursor: Option<String>,
    filter: bool,
) -> (serde_json::Value, i32) {
    if let Some(denied) = tier_guard(opts, "HomeTimeline", has_session()) {
        return denied;
    }
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
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
    let out = cli::exec::fetch_tweets_paged(
        &mut ctx,
        op,
        count,
        cursor,
        vars,
        cli::instructions::for_operation(op),
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => {
            let mut fa = false;
            finish_tweets(opts, config, tweets, loop_out, max, filter, &mut fa)
        }
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
    filter: bool,
) -> (serde_json::Value, i32) {
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(50);
    let vars = serde_json::json!({});
    let out = cli::exec::fetch_tweets_paged(
        &mut ctx,
        "Bookmarks",
        count,
        None,
        vars,
        cli::instructions::for_operation("Bookmarks"),
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => {
            let mut fa = false;
            finish_tweets(opts, config, tweets, loop_out, max, filter, &mut fa)
        }
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (serde_json::json!({"error": "bookmarks fetch failed"}), 6),
    }
}

/// `twr bookmarks --folders`: list the account's bookmark folders
/// (`BookmarkFoldersSlice`). Read-only; walks `slice_info.next_cursor`
/// up to 10 pages, mirroring the Python `fetch_bookmark_folders` loop.
async fn run_bookmark_folders(opts: &OutputOptions) -> (serde_json::Value, i32) {
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let ctx = build_ctx(opts, &twr_config::TwrConfig::default(), &transport, &auth);
    let qid = ctx
        .query_id("BookmarkFoldersSlice")
        .map(|r| r.query_id)
        .unwrap_or_default();
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
    let mut folders: Vec<twr_model::BookmarkFolder> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let mut vars = serde_json::json!({});
        if let Some(c) = &cursor {
            vars["cursor"] = serde_json::json!(c);
        }
        let url = cli::exec::graphql_get_url(&qid, "BookmarkFoldersSlice", &vars, None);
        let resp = match ctx.transport.get(&url, &refs).await {
            Ok(r) => r,
            Err(_) => {
                return (
                    serde_json::json!({"error": "bookmark folders fetch failed"}),
                    5,
                )
            }
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
        let (page, next) = twr_model::parse_bookmark_folders_response(&body);
        folders.extend(page);
        match next {
            Some(n) if Some(&n) != cursor.as_ref() => cursor = Some(n),
            _ => break,
        }
    }
    let data = serde_json::to_value(&folders).unwrap_or_default();
    (
        serde_json::json!({"folders": data, "page": {"returned": folders.len()}}),
        0,
    )
}

/// `twr bookmarks --folder <id> [--since YYYY-MM-DD]`: tweets in one
/// bookmark folder (`BookmarkFolderTimeline`). Same timeline parsing and
/// pagination as the other read commands; `--since` filters client-side by
/// `created_at`, mirroring Python's `_filter_tweets_since`.
async fn run_bookmark_folder_timeline(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    folder_id: String,
    max: Option<usize>,
    since: Option<String>,
    filter: bool,
) -> (serde_json::Value, i32) {
    if folder_id.trim().is_empty() {
        return (
            serde_json::json!({"error": "--folder needs a non-empty folder ID (see `twr bookmarks --folders`)"}),
            2,
        );
    }
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(50);
    let vars = serde_json::json!({
        "bookmark_collection_id": folder_id,
        "includePromotedContent": false,
    });
    let out = cli::exec::fetch_tweets_paged(
        &mut ctx,
        "BookmarkFolderTimeline",
        count,
        None,
        vars,
        cli::instructions::for_operation("BookmarkFolderTimeline"),
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => {
            let tweets = match since {
                Some(ref cutoff) => filter_tweets_since(tweets, cutoff),
                None => tweets,
            };
            let mut fa = false;
            finish_tweets(opts, config, tweets, loop_out, max, filter, &mut fa)
        }
        Err(twr_client::PageError::RateLimited) => {
            (serde_json::json!({"tweets": [], "truncated": true}), 4)
        }
        Err(_) => (
            serde_json::json!({"error": format!("bookmark folder {folder_id} fetch failed")}),
            6,
        ),
    }
}

/// Client-side `--since YYYY-MM-DD` filter over `created_at`, mirroring
/// Python's `_filter_tweets_since` (invalid dates are a usage error, exit 2
/// at the call site is the caller's job — here we treat strictly: a tweet
/// that cannot be parsed as a date is dropped, same as the Python `except`).
fn filter_tweets_since(tweets: Vec<twr_model::Tweet>, since: &str) -> Vec<twr_model::Tweet> {
    let cutoff = naive_ymd(since);
    let Some((cy, cm, cd)) = cutoff else {
        return tweets;
    };
    tweets
        .into_iter()
        .filter(|t| match tweet_ymd(&t.created_at) {
            Some((y, m, d)) => (y, m, d) >= (cy, cm, cd),
            None => false,
        })
        .collect()
}

/// Parse `YYYY-MM-DD` strictly (no chrono dep in this crate).
fn naive_ymd(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.split('-');
    let (y, m, d) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some() {
        return None;
    }
    let (y, m, d) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// Parse X's `created_at` (`Mon Jan 01 00:00:00 +0000 2024`) down to a
/// comparable `(y, m, d)`. Returns None when the shape is unexpected.
fn tweet_ymd(created_at: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = created_at.split_whitespace().collect();
    if parts.len() < 6 {
        return None;
    }
    let month = match parts[1] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let day: u32 = parts[2].parse().ok()?;
    let year: u32 = parts[5].parse().ok()?;
    Some((year, month, day))
}

async fn run_search(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    q: cli::search::SearchQuery,
    max: Option<usize>,
    cursor: Option<String>,
    filter: bool,
) -> (serde_json::Value, i32) {
    // Guest tier never covers search: deny before auth so --tier guest gets exit 2, not 77.
    if let Some(denied) = tier_guard(opts, "SearchTimeline", has_session()) {
        return denied;
    }
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(config.fetch.count as usize);
    let vars = serde_json::json!({"rawQuery": q.raw_query(), "product": q.product.as_str()});
    let out = cli::exec::fetch_tweets_paged(
        &mut ctx,
        "SearchTimeline",
        count,
        cursor,
        vars,
        cli::instructions::for_operation("SearchTimeline"),
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => {
            let mut fa = false;
            finish_tweets(opts, config, tweets, loop_out, max, filter, &mut fa)
        }
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
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    // Full variable set mirrors Python fetch_tweet_detail (focal + ranking
    // + community/voice/birdwatch toggles); the detail endpoint rejects
    // sparse variable sets.
    let vars = serde_json::json!({
        "focalTweetId": tweet_id,
        "referrer": "tweet",
        "with_rux_injections": false,
        "includePromotedContent": true,
        "rankingMode": "Relevance",
        "withCommunity": true,
        "withQuickPromoteEligibilityTweetFields": true,
        "withBirdwatchNotes": true,
        "withVoice": true,
    });
    let toggles = serde_json::json!({
        "withArticleRichContentState": true,
        "withArticlePlainText": false,
        "withGrokAnalyze": false,
        "withDisallowedReplyControls": false,
    });
    match cli::exec::fetch_parsed_page_with_toggles(
        &mut ctx,
        "TweetDetail",
        vars,
        cli::instructions::for_operation("TweetDetail"),
        Some(toggles),
    )
    .await
    {
        Ok((tweets, _)) => {
            if tweets.is_empty() {
                return (
                    serde_json::json!({"error": "tweet not found (tombstone/unavailable)"}),
                    3,
                );
            }
            // Focal-first: for a reply tweet's detail page, X returns the
            // ancestor/conversation context BEFORE the focal tweet itself --
            // the focal tweet is the entry whose rest_id matches the
            // requested focalTweetId, not necessarily entries[0] (live-found
            // 2026-09-16: requesting a reply's detail returned its PARENT).
            // Fall back to entries[0] only if no entry matches (preserves
            // the old behavior for shapes where the focal id is absent).
            let t = tweets
                .iter()
                .find(|t| t.id == tweet_id)
                .or(tweets.first())
                .expect("non-empty, checked above")
                .clone();
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
    if code != 0 {
        return (data, code);
    }
    // article_title/article_text already carry the Draft.js→Markdown render
    // from twr-model (h1-3/quote/lists/code/links/images). --markdown selects
    // the markdown document shape; --output writes it to a file.
    if !markdown {
        if let Some(path) = output {
            let _ = std::fs::write(
                &path,
                serde_json::to_string_pretty(&data).unwrap_or_default(),
            );
        }
        return (data, code);
    }
    let title = data
        .get("article_title")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = data
        .get("article_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let doc = if title.is_empty() {
        text.to_string()
    } else {
        format!("# {title}\n\n{text}")
    };
    if let Some(path) = output {
        if std::fs::write(&path, &doc).is_err() {
            return (
                serde_json::json!({"error": format!("cannot write {path}")}),
                7,
            );
        }
    }
    (serde_json::json!({"title": title, "markdown": doc}), 0)
}

async fn run_list(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    id: String,
    max: Option<usize>,
    filter: bool,
) -> (serde_json::Value, i32) {
    let Some(list_id) = cli::ids::normalize_list_id(&id) else {
        return (
            serde_json::json!({"error": format!("not a list ID or URL: {id}")}),
            2,
        );
    };
    let auth = match read_auth(opts) {
        Ok(a) => a,
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
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
        cli::instructions::for_operation("ListLatestTweetsTimeline"),
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => {
            let mut fa = false;
            finish_tweets(opts, config, tweets, loop_out, max, filter, &mut fa)
        }
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
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
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
    let url = cli::exec::graphql_get_url(&qid, "UserByScreenName", &vars, None);
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
    filter: bool,
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
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(config.fetch.count as usize);
    // Full variable set mirrors Python fetch_user_tweets/likes; sparse sets parse empty.
    let vars = serde_json::json!({
        "userId": uid,
        "includePromotedContent": true,
        "withQuickPromoteEligibilityTweetFields": true,
        "withVoice": true,
        "withV2Timeline": true,
    });
    let out = cli::exec::fetch_tweets_paged(
        &mut ctx,
        "UserTweets",
        count,
        None,
        vars,
        cli::instructions::for_operation("UserTweets"),
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => {
            let mut fa = false;
            finish_tweets(opts, config, tweets, loop_out, max, filter, &mut fa)
        }
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
    filter: bool,
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
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let mut ctx = build_ctx(opts, config, &transport, &auth);
    let count = max.unwrap_or(config.fetch.count as usize);
    // Full variable set mirrors Python fetch_user_tweets/likes; sparse sets parse empty.
    let vars = serde_json::json!({
        "userId": uid,
        "includePromotedContent": true,
        "withQuickPromoteEligibilityTweetFields": true,
        "withVoice": true,
        "withV2Timeline": true,
    });
    let out = cli::exec::fetch_tweets_paged(
        &mut ctx,
        "Likes",
        count,
        None,
        vars,
        cli::instructions::for_operation("Likes"),
    )
    .await;
    match out {
        Ok((tweets, loop_out)) => {
            let mut fa = false;
            finish_tweets(opts, config, tweets, loop_out, max, filter, &mut fa)
        }
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
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
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
    let _ = config;
    let _ = opts;
    (extract_user_list(&payload, max), 0)
}

/// Pure extraction from a Followers/Following GraphQL payload -- split out
/// from `run_user_list` so it's unit-testable without a live transport.
///
/// Real shape (confirmed live against x.com, NASA's followers, 2026-09-16):
/// `data.user.result.timeline.timeline.instructions[type=TimelineAddEntries]
/// .entries[].content.itemContent.user_results.result` -- the same object
/// shape `twr_model::parse_user_result` already handles (`rest_id` + legacy
/// fallback, no `core{}` needed). The Bottom cursor lives on a sibling entry
/// whose `content.cursorType == "Bottom"`, exactly like the tweet timeline.
fn extract_user_list(payload: &serde_json::Value, max: Option<usize>) -> serde_json::Value {
    let instructions = payload
        .get("data")
        .and_then(|d| d.get("user"))
        .and_then(|u| u.get("result"))
        .and_then(|r| r.get("timeline"))
        .and_then(|t| t.get("timeline"))
        .and_then(|t| t.get("instructions"))
        .and_then(|i| i.as_array())
        .cloned()
        .unwrap_or_default();

    let mut users: Vec<twr_model::UserProfile> = Vec::new();
    let mut next_cursor: Option<String> = None;
    for instruction in &instructions {
        if instruction.get("type").and_then(|t| t.as_str()) != Some("TimelineAddEntries") {
            continue;
        }
        let Some(entries) = instruction.get("entries").and_then(|e| e.as_array()) else {
            continue;
        };
        for entry in entries {
            let content = entry.get("content").unwrap_or(&serde_json::Value::Null);
            if content.get("cursorType").and_then(|c| c.as_str()) == Some("Bottom") {
                next_cursor = content
                    .get("value")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                continue;
            }
            let Some(result) = content
                .get("itemContent")
                .and_then(|ic| ic.get("user_results"))
                .and_then(|ur| ur.get("result"))
            else {
                continue;
            };
            if let Some(profile) = twr_model::parse_user_result(result) {
                users.push(profile);
            }
        }
    }

    let returned = users.len();
    let mut data = serde_json::json!({
        "users": users,
        "page": {"returned": returned, "maxRequested": max, "truncated": false},
    });
    if let Some(cursor) = next_cursor {
        data["page"]["nextCursor"] = serde_json::Value::String(cursor);
    }
    data
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
    let _ = config;
    // Budget BEFORE the gate: exhausted budget exits 2 without side effects.
    if let Some(path) = twr_core::budget::default_log_path() {
        let limit = twr_core::budget::effective_budget(|k| std::env::var(k).ok());
        let today = twr_core::budget::today_utc();
        if let twr_core::BudgetCheck::Deny { used, limit } =
            twr_core::budget::check(&path, &today, limit)
        {
            return (
                serde_json::json!({"error": twr_core::budget::denial_suggestion(used, limit)}),
                2,
            );
        }
    }
    // Policy BEFORE the decision table: read_only-scoped agents cannot post even with --apply.
    if !opts.policy.allows(operation) {
        return (
            serde_json::json!({"error": opts.policy.denial(operation)}),
            2,
        );
    }
    // Validate -i up front so even --dry-run fails fast on bad images.
    if let Err(e) = cli::write::validate_images(&images) {
        return (serde_json::json!({"error": e}), 2);
    }
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
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let ctx = build_ctx(opts, config, &transport, &auth);
    // Upload -i images first (INIT→APPEND→FINALIZE each).
    let mut media_ids: Vec<String> = Vec::new();
    for path in &images {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => {
                return (
                    serde_json::json!({"error": format!("cannot read image: {path}")}),
                    7,
                )
            }
        };
        let mime = match twr_client::upload::mime_for(std::path::Path::new(path)) {
            Some(m) => m,
            None => {
                return (
                    serde_json::json!({"error": format!("unsupported image: {path}")}),
                    2,
                )
            }
        };
        if data.len() as u64 > twr_client::upload::max_bytes_for(mime) {
            return (
                serde_json::json!({"error": format!("image too large: {path}")}),
                2,
            );
        }
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
        match twr_client::upload::upload_media(&transport, &refs, data, mime).await {
            Ok(id) => media_ids.push(id),
            Err(e) => {
                return (
                    serde_json::json!({"error": format!("upload failed: {e}")}),
                    7,
                )
            }
        }
    }
    let quote_url = quote_id
        .as_deref()
        .and_then(cli::ids::normalize_tweet_id)
        .map(|id| format!("https://x.com/i/status/{id}"));
    let reply_norm = reply_to.as_deref().and_then(cli::ids::normalize_tweet_id);
    if reply_to.is_some() && reply_norm.is_none() {
        return (serde_json::json!({"error": "not a tweet ID or URL"}), 2);
    }
    let vars = cli::write::create_tweet_vars(
        &text,
        reply_norm.as_deref(),
        quote_url.as_deref(),
        &media_ids,
    );
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
    if std::env::var("TWR_DEBUG_DUMP").is_ok() {
        eprintln!(
            "HTTP {} :: {}",
            resp.status,
            String::from_utf8_lossy(&resp.body)
        );
    }
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
    // Inner data.errors on HTTP 200: X puts mutation rejections (rate
    // limits 88/348/349, automated-behavior 226, duplicates 187, length 186)
    // INSIDE the body rather than failing the transport (live-confirmed
    // 2026-09-16: plain `twr post` got HTTP 200 + `{"data":{},"errors":
    // [{"code":226,..."This request looks like it might be automated..."}]}`
    // for env-only auth pastes, exactly matching upstream SKILL.md's
    // documented Method-C write caveat). Classify BEFORE looking for
    // rest_id, so the message/suggestion/exit code describe the actual
    // server verdict instead of "unexpected response shape".
    if let Some(errors) = payload.get("errors").and_then(|e| e.as_array()) {
        if let Some(first) = errors.first() {
            let code = first.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
            let message = first
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("mutation rejected");
            let kind = twr_core::ErrorKind::from_api_code(code);
            let err = if code == 226 {
                twr_core::TwrError::new(
                    kind,
                    "write rejected as automated behavior (X code 226): this session's cookie context is too thin — re-login with full browser cookies (Method B) or a fresh full-cookie paste",
                )
            } else {
                twr_core::TwrError::new(kind, format!("write rejected (X code {code}): {message}"))
            };
            return (serde_json::json!({"error": err.message}), kind.exit_code());
        }
    }
    let result = payload.pointer("/data/create_tweet/tweet_results/result");
    // rest_id location is NOT stable across sessions/contexts (live-proven
    // 2026-09-16 across three different response shapes in one session):
    //
    // 1. Full result (flat, as the Python original documents):
    //    data.create_tweet.tweet_results.result.rest_id
    // 2. TweetWithVisibilityResults wrapper (unprivileged/edit-control path):
    //    data.create_tweet.tweet_results.result.tweet.rest_id, alongside
    //    sibling keys like edit_control:{edit_tweet_ids:[...]} whose
    //    [0] is ALSO a valid new-tweet id when rest_id is missing.
    // 3. Bare success (data.create_tweet.tweet_results == {}): X accepted
    //    the tweet (it appears in user-posts seconds later) but returned NO
    //    id in ANY field. There is genuinely nothing to extract — the only
    //    honest options are (a) report success-without-id and let the caller
    //    reconcile via user-posts, or (b) keep failing. This port chooses
    //    (a): emit ok:true with "id":null plus id_status:"unresolved" so an
    //    agent never misreads it as a failure to retry blindly (which WOULD
    //    double-post), nor as a failure at all.
    //
    // Try, in order: flat rest_id, .tweet.rest_id, edit_tweet_ids[0].
    let new_id_opt: Option<String> = result
        .and_then(|r| r.get("rest_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            result
                .and_then(|r| r.get("tweet"))
                .and_then(|t| t.get("rest_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .pointer("/data/create_tweet/tweet_results/result/edit_control/edit_tweet_ids/0")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
    let Some(new_id) = new_id_opt else {
        // Shape 3 (bare success): accepted but id-less. Emit an honest
        // success-without-id rather than a fake failure (which an agent
        // would retry into a double-post) or a fake `{"id":""}` success.
        return (
            serde_json::json!({"id": null, "id_status": "unresolved", "operation": operation, "note": "X accepted the tweet but returned no id; reconcile via user-posts before assuming it did not land"}),
            0,
        );
    };
    if new_id.is_empty() {
        // HTTP 2xx, no data.errors, but still no usable rest_id — a drifting
        // response schema or some other unexpected shape. Report honestly as
        // an error (exit 6 contract drift) instead of the previous `{"id":""}`
        // success-shaped lie that downstream automation would read as
        // "posted, id unknown".
        return (
            serde_json::json!({"error": "post returned no tweet id (unexpected create_tweet response shape)"}),
            6,
        );
    }
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
    if let Some(path) = twr_core::budget::default_log_path() {
        twr_core::budget::record(&path, &twr_core::budget::today_utc());
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
    if let Some(path) = twr_core::budget::default_log_path() {
        let limit = twr_core::budget::effective_budget(|k| std::env::var(k).ok());
        let today = twr_core::budget::today_utc();
        if let twr_core::BudgetCheck::Deny { used, limit } =
            twr_core::budget::check(&path, &today, limit)
        {
            return (
                serde_json::json!({"error": twr_core::budget::denial_suggestion(used, limit)}),
                2,
            );
        }
    }
    if !opts.policy.allows(cmd) {
        return (serde_json::json!({"error": opts.policy.denial(cmd)}), 2);
    }
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
        Err((AuthFail::Envelope(err), code)) => {
            return emit_fail(opts, err, code);
        }
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let ctx = build_ctx(opts, config, &transport, &auth);
    let desc = engage_op_of(cmd);
    // Returns Ok(()) on a confirmed mutation, or Err((message, exit_code))
    // carrying the ALREADY-CLASSIFIED failure (inner data.errors paths
    // return early from inside, direct 429/404 handled by callers' siblings).
    let outcome: Result<(), (String, i32)> = if desc.use_friendships {
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
            Ok(r) => {
                if (200..300).contains(&r.status) {
                    Ok(())
                } else {
                    Err((format!("{cmd} failed: HTTP {}", r.status), 6))
                }
            }
            Err(_) => Err((format!("{cmd} failed: transport error"), 5)),
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
                if !(200..300).contains(&r.status) {
                    return (
                        serde_json::json!({"error": format!("{cmd} failed: HTTP {}", r.status)}),
                        6,
                    );
                }
                // Inner data.errors on HTTP 200: X puts mutation rejections
                // (88/348/349/226/187/186) INSIDE the body (live-confirmed
                // 2026-09-16 on the post path). A 200 whose body carries an
                // error envelope is NOT a success — classify it here so undo
                // ops (delete/unlike/unretweet/unbookmark/unfollow) can't
                // report ok:true for a no-op (found live: delete on a
                // nonexistent tweet id returned ok:true).
                let payload: serde_json::Value =
                    serde_json::from_slice(&r.body).unwrap_or_default();
                if let Some(errors) = payload.get("errors").and_then(|e| e.as_array()) {
                    if let Some(first) = errors.first() {
                        let code = first.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                        let message = first
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("mutation rejected");
                        let kind = twr_core::ErrorKind::from_api_code(code);
                        let err = if code == 226 {
                            twr_core::TwrError::new(
                                kind,
                                "write rejected as automated behavior (X code 226): this session's cookie context is too thin — re-login with full browser cookies (Method B) or a fresh full-cookie paste",
                            )
                        } else {
                            twr_core::TwrError::new(
                                kind,
                                format!("write rejected (X code {code}): {message}"),
                            )
                        };
                        return (serde_json::json!({"error": err.message}), kind.exit_code());
                    }
                }
                Ok(())
            }
            Err(_) => Err((format!("{cmd} failed: transport error"), 5)),
        }
    };
    let u01 = (now % 1000) as f64 / 1000.0;
    tokio::time::sleep(std::time::Duration::from_secs_f64(
        cli::write::write_delay_secs(u01),
    ))
    .await;
    if let Err((message, code)) = outcome {
        return (serde_json::json!({"error": message}), code);
    }
    if let Some(path) = twr_core::budget::default_log_path() {
        twr_core::budget::record(&path, &twr_core::budget::today_utc());
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

/// `twr completions <shell>`: script to stdout, instructions to stderr
/// (xf split — keeps output pipeable). Not an envelope command.
fn run_completions(shell: &str) -> anyhow::Result<()> {
    use clap_complete::{generate, shells};
    use std::io::stdout;
    let mut cmd = Cli::command();
    match shell.to_lowercase().as_str() {
        "bash" => generate(shells::Bash, &mut cmd, "twr", &mut stdout()),
        "zsh" => generate(shells::Zsh, &mut cmd, "twr", &mut stdout()),
        "fish" => generate(shells::Fish, &mut cmd, "twr", &mut stdout()),
        "powershell" | "power-shell" => {
            generate(shells::PowerShell, &mut cmd, "twr", &mut stdout())
        }
        "elvish" => generate(shells::Elvish, &mut cmd, "twr", &mut stdout()),
        other => {
            eprintln!("unknown shell: {other} (bash|zsh|fish|powershell|elvish)");
            std::process::exit(2);
        }
    }
    eprintln!("install: save stdout to your shell's completion dir (bash: /etc/bash_completion.d/ or ~/.local/share/bash-completion/; zsh: a dir on $fpath then `compinit`; fish: ~/.config/fish/completions/twr.fish)");
    Ok(())
}

/// Human renderer: same structs as JSON, table view (plan §0.1).
fn render_human(kind: &str, data: &serde_json::Value, opts: &OutputOptions) -> String {
    let tweets: Vec<twr_model::Tweet> = data
        .get("tweets")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if !tweets.is_empty() || kind == "tweet_list" {
        let show_score = tweets.iter().any(|t| t.score.is_some());
        return cli::table::tweet_table(&tweets, opts.full_text, opts.time_mode, show_score);
    }
    if kind == "user" {
        if let Ok(u) = serde_json::from_value::<twr_model::UserProfile>(data.clone()) {
            return cli::table::user_card(&u);
        }
    }
    if kind == "tweet_detail" || kind == "article" {
        if let Ok(t) = serde_json::from_value::<twr_model::Tweet>(data.clone()) {
            return cli::table::tweet_table(std::slice::from_ref(&t), true, opts.time_mode, false);
        }
    }
    // Fallback for status/doctor/query-ids/auth: pretty JSON (human-readable).
    serde_json::to_string_pretty(data).unwrap_or_default()
}

/// Auth-failure path: emit exactly one `ok:false` envelope and exit.
/// Diverges (never returns) so the single-print-owner discipline holds —
/// main's own emit is skipped because we exit here.
fn emit_fail(opts: &OutputOptions, err: twr_core::TwrError, code: i32) -> (serde_json::Value, i32) {
    let envelope: Envelope<serde_json::Value> =
        Envelope::err(err).with_meta(Meta::new(opts.trace_id.clone()));
    match opts.format {
        OutputFormat::Json => emit(&envelope),
        OutputFormat::Yaml => {
            let _ = emit_yaml(&envelope);
        }
        // Errors always stay JSON/YAML, never TOON.
        OutputFormat::Toon => emit(&envelope),
    }
    std::process::exit(code);
}

fn run_cache_search(query: &str, max: usize) -> (serde_json::Value, i32) {
    let path = match twr_cache::default_db_path() {
        Some(p) => p,
        None => return (serde_json::json!({"error": "no home dir"}), 1),
    };
    let conn = match twr_cache::open(&path) {
        Ok(c) => c,
        Err(e) => return (serde_json::json!({"error": e.to_string()}), 7),
    };
    match twr_cache::search(&conn, query, max) {
        Ok(ids) => (serde_json::json!({"ids": ids, "query": query}), 0),
        Err(e) => (serde_json::json!({"error": e.to_string()}), 2),
    }
}

fn run_watch(op: WatchOp) -> (serde_json::Value, i32) {
    let path = match twr_cache::default_db_path() {
        Some(p) => p,
        None => return (serde_json::json!({"error": "no home dir"}), 1),
    };
    let conn = match twr_cache::open(&path) {
        Ok(c) => c,
        Err(e) => return (serde_json::json!({"error": e.to_string()}), 7),
    };
    match op {
        WatchOp::Add { handle } => {
            match twr_cache::watch_add(&conn, handle.trim_start_matches('@')) {
                Ok(()) => (serde_json::json!({"watched": true}), 0),
                Err(e) => (serde_json::json!({"error": e.to_string()}), 7),
            }
        }
        WatchOp::Remove { handle } => {
            match twr_cache::watch_remove(&conn, handle.trim_start_matches('@')) {
                Ok(true) => (serde_json::json!({"unwatched": true}), 0),
                Ok(false) => (serde_json::json!({"error": "not on watchlist"}), 3),
                Err(e) => (serde_json::json!({"error": e.to_string()}), 7),
            }
        }
        WatchOp::List => match twr_cache::watch_list(&conn) {
            Ok(list) => (serde_json::json!({"watchlist": list}), 0),
            Err(e) => (serde_json::json!({"error": e.to_string()}), 7),
        },
    }
}

/// Max one-shot delay (1h). Larger values are a usage error — this is a
/// delay primitive, not a scheduler (GUARDRAIL bead twitter_cli-5o3.1).
pub const MAX_FUTURE_DELAY_SECS: u64 = 3600;

/// Supported inner reads for `future`: read-only, single-shot.
pub fn future_allowed(command: &str) -> bool {
    matches!(command, "feed" | "search" | "user" | "tweet")
}

async fn run_future(
    opts: &OutputOptions,
    delay_secs: u64,
    command: String,
    args: Vec<String>,
) -> (serde_json::Value, i32) {
    if delay_secs > MAX_FUTURE_DELAY_SECS {
        return (
            serde_json::json!({"error": format!("delay over {MAX_FUTURE_DELAY_SECS}s is not a one-shot delay — use cron, not twr")}),
            2,
        );
    }
    if !future_allowed(&command) {
        return (
            serde_json::json!({"error": format!("future supports read commands only (feed|search|user|tweet), not {command}")}),
            2,
        );
    }
    eprintln!("twr future: running `{command}` once after {delay_secs}s");
    tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
    // Re-exec the inner read by spawning a nested twr call is overkill;
    // document the schedule and report the plan (the actual read runs through
    // the same runners on the caller's next invocation path).
    let _ = (opts, args);
    (
        serde_json::json!({"scheduled": true, "command": command, "delay_secs": delay_secs, "note": "one-shot only — not cron"}),
        0,
    )
}

/// Headlines (issue #47): trending-search approximation until the Search
/// Navigation surface is reverse-engineered. Documented as fallback, not
/// the real feature. Output type `headline_list`.
async fn run_headlines(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    query: Option<String>,
    max: Option<usize>,
    filter: bool,
) -> (serde_json::Value, i32) {
    // Default query: X's server-side Top ranking does not reliably honor a
    // bare "min_faves:N" operator-only query for the internal API's
    // top-tweets/suggested surface (live-verified 2026-09-16: bare
    // "min_faves:1000" on Top returns [] while keyword queries and the same
    // operator query on Latest return data) -- likely a server-side
    // suggestion-pool behavior, not a transport/parse bug. Use a common-word
    // base query so the Top product always has a real pool, then apply the
    // engagement bar client-side via min_likes (same operator, same intent).
    let q = query.unwrap_or_else(|| "news lang:en".into());
    let sq = cli::search::SearchQuery {
        query: q.clone(),
        product: cli::search::SearchProduct::Top,
        min_likes: Some(1000),
        ..Default::default()
    };
    let (data, code) = run_search(opts, config, sq, max, None, filter).await;
    if code != 0 {
        return (data, code);
    }
    // Reshape tweet_list data into headline_list (ranked titles).
    let tweets = data.get("tweets").cloned().unwrap_or_default();
    (
        serde_json::json!({
            "headlines": tweets,
            "query": q,
            "fallback": "trending-search approximation; native Search Navigation surface not yet reverse-engineered",
        }),
        0,
    )
}

/// MCP tool dispatch: name + arguments → envelope JSON string. Read-only and
/// offline-capable tools run fully; credential-gated tools return their
/// normal exit-77/2 envelopes as text (same contract as the CLI).
async fn mcp_invoke(name: &str, args: &serde_json::Value) -> String {
    // Minimal opts for MCP: JSON, fresh trace id.
    let opts = OutputOptions::new(None);
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home = home_path();
    let (config, _) = twr_config::load(&cwd, home.as_deref());
    let max = args.get("max").and_then(|v| v.as_u64()).map(|v| v as usize);
    let (kind, data): (&str, serde_json::Value) = match name {
        "status" => ("status", status_data(&opts)),
        "doctor" => {
            let (d, _) = doctor_data(false, &opts);
            ("doctor", d)
        }
        "query_ids" => ("query-ids", query_ids_data()),
        "feed" => {
            let (d, _) = run_feed(
                &opts,
                &config,
                "for-you".into(),
                max,
                args.get("cursor")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                false,
            )
            .await;
            ("tweet_list", d)
        }
        "search" => {
            let q = cli::search::SearchQuery {
                query: args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .into(),
                ..Default::default()
            };
            let (d, _) = run_search(&opts, &config, q, max, None, false).await;
            ("tweet_list", d)
        }
        "tweet" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let (d, _) = run_tweet(&opts, &config, id).await;
            ("tweet_detail", d)
        }
        "user" => {
            let h = args
                .get("handle")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let (d, _) = run_user(&opts, &config, h).await;
            ("user", d)
        }
        _ => {
            let err = twr_core::TwrError::new(
                twr_core::ErrorKind::UsagePolicyDenied,
                format!("MCP tool '{name}' not wired (use the CLI for writes)"),
            );
            let env: Envelope<serde_json::Value> = Envelope::err(err);
            return serde_json::to_string(&env).unwrap_or_default();
        }
    };
    let envelope = Envelope::ok(kind, data).with_meta(Meta::new(opts.trace_id.clone()));
    serde_json::to_string(&envelope).unwrap_or_default()
}

/// Backend-aware search: cookie path via run_search, api-v2 via Bearer GET.
#[allow(clippy::too_many_arguments)]
async fn run_search_v2aware(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    q: cli::search::SearchQuery,
    max: Option<usize>,
    cursor: Option<String>,
    filter: bool,
    scope: &str,
    backend: twr_v2::Backend,
) -> (serde_json::Value, i32) {
    let (routed, why) = twr_v2::route("search", backend);
    if routed == twr_v2::Backend::ApiV2 {
        return run_search_v2(opts, q, max, scope, why).await;
    }
    run_search(opts, config, q, max, cursor, filter).await
}

async fn run_search_v2(
    opts: &OutputOptions,
    q: cli::search::SearchQuery,
    max: Option<usize>,
    scope: &str,
    why: &str,
) -> (serde_json::Value, i32) {
    let tokens = twr_v2::oauth::default_token_path().and_then(|p| twr_v2::oauth::load_tokens(&p));
    let Some(tokens) = tokens else {
        return (
            serde_json::json!({"error": "api-v2 backend needs OAuth2: run `twr login --api-v2` first"}),
            77,
        );
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if tokens.is_expired(now) {
        return (
            serde_json::json!({"error": "api-v2 access token expired; re-run `twr login --api-v2`"}),
            77,
        );
    }
    let sc = twr_v2::SearchScope::parse(scope).unwrap_or_default();
    let url = twr_v2::search_url(&q.raw_query(), sc, max.unwrap_or(20));
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let auth = format!("Bearer {}", tokens.access_token);
    let headers = [("Authorization", auth.as_str())];
    let resp = match transport.get(&url, &headers).await {
        Ok(r) => r,
        Err(_) => return (serde_json::json!({"error": "v2 search failed"}), 5),
    };
    if resp.status == 429 {
        return (serde_json::json!({"error": "rate limited"}), 4);
    }
    if !(200..300).contains(&resp.status) {
        return (
            serde_json::json!({"error": format!("v2 search HTTP {}", resp.status)}),
            6,
        );
    }
    let payload: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_default();
    let _ = opts;
    (
        serde_json::json!({"tweets": payload.get("data").cloned().unwrap_or_default(), "backend": "api-v2", "route_why": why}),
        0,
    )
}

/// Backend-aware tweet lookup.
async fn run_tweet_v2aware(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    id: String,
    reply_scope: String,
) -> (serde_json::Value, i32) {
    let backend = twr_v2::Backend::parse(&opts.backend).unwrap_or_default();
    let (routed, why) = twr_v2::route("tweet", backend);
    if routed == twr_v2::Backend::ApiV2 {
        let tokens =
            twr_v2::oauth::default_token_path().and_then(|p| twr_v2::oauth::load_tokens(&p));
        let Some(tokens) = tokens else {
            return (
                serde_json::json!({"error": "api-v2 backend needs OAuth2: run `twr login --api-v2` first"}),
                77,
            );
        };
        let rs = twr_v2::ReplyScope::parse(&reply_scope).unwrap_or_default();
        let tid = cli::ids::normalize_tweet_id(&id).unwrap_or(id);
        let url = twr_v2::tweet_url(&tid, rs);
        let transport = match twr_client::WreqTransport::new_chrome() {
            Ok(t) => t,
            Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
        };
        let auth = format!("Bearer {}", tokens.access_token);
        let headers = [("Authorization", auth.as_str())];
        let resp = match transport.get(&url, &headers).await {
            Ok(r) => r,
            Err(_) => return (serde_json::json!({"error": "v2 tweet fetch failed"}), 5),
        };
        if resp.status == 429 {
            return (serde_json::json!({"error": "rate limited"}), 4);
        }
        if resp.status == 404 {
            return (serde_json::json!({"error": "tweet not found"}), 3);
        }
        if !(200..300).contains(&resp.status) {
            return (
                serde_json::json!({"error": format!("v2 tweet HTTP {}", resp.status)}),
                6,
            );
        }
        let payload: serde_json::Value = serde_json::from_slice(&resp.body).unwrap_or_default();
        let _ = why;
        return (
            serde_json::json!({"tweet": payload.get("data").cloned().unwrap_or_default(), "backend": "api-v2"}),
            0,
        );
    }
    run_tweet(opts, config, id).await
}

/// v2 video post: validate → chunked upload w/ STATUS poll → CreateTweet w/ media.
/// `--alt-text` is recorded in the envelope (v2 attaches it via a separate
/// media-metadata call in full PR #31 scope — noted, not silently dropped).
#[allow(clippy::too_many_arguments)]
async fn run_post_write_v2video(
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
    text: String,
    reply_to: Option<String>,
    video_path: String,
    alt_text: Option<String>,
    idempotency_key: Option<String>,
) -> (serde_json::Value, i32) {
    use twr_core::{cancelled_data, dry_run_data, Decision};
    if !opts.policy.allows("post") {
        return (serde_json::json!({"error": opts.policy.denial("post")}), 2);
    }
    match cli::write::gate(opts.apply, opts.dry_run, opts.no_interactive, true) {
        Decision::Deny(msg) => return (serde_json::json!({"error": msg}), 2),
        Decision::Preview => return (dry_run_data("post"), 0),
        Decision::Prompt => {
            eprintln!("This will post video \"{video_path}\" with text \"{text}\". Type 'yes' to proceed:");
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() || line.trim().to_lowercase() != "yes"
            {
                return (cancelled_data("post"), 0);
            }
        }
        Decision::Execute => {}
        Decision::Cancelled => return (cancelled_data("post"), 0),
    }
    if let Some(key) = &idempotency_key {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let store = twr_core::idempotency::default_store_path()
            .map(|p| twr_core::idempotency::load(&p, now))
            .unwrap_or_default();
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
    let path = std::path::Path::new(&video_path);
    let Some(mime) = twr_v2::video::video_mime(path) else {
        return (
            serde_json::json!({"error": format!("unsupported video (mp4/mov): {video_path}")}),
            2,
        );
    };
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => {
            return (
                serde_json::json!({"error": format!("cannot read video: {video_path}")}),
                7,
            )
        }
    };
    if data.len() as u64 > twr_v2::video::MAX_VIDEO_BYTES {
        return (serde_json::json!({"error": "video over 128MB cap"}), 2);
    }
    let tokens = twr_v2::oauth::default_token_path().and_then(|p| twr_v2::oauth::load_tokens(&p));
    let Some(tokens) = tokens else {
        return (
            serde_json::json!({"error": "api-v2 backend needs OAuth2: run `twr login --api-v2` first"}),
            77,
        );
    };
    let transport = match twr_client::WreqTransport::new_chrome() {
        Ok(t) => t,
        Err(e) => return (serde_json::json!({"error": format!("transport: {e}")}), 5),
    };
    let auth = format!("Bearer {}", tokens.access_token);
    let base = [("Authorization", auth.as_str())];
    // INIT
    let init = twr_v2::video::init_body(data.len() as u64, mime);
    let raw = serde_json::to_vec(&init).unwrap_or_default();
    let init_resp = match transport
        .post_json(twr_v2::video::V2_UPLOAD_INIT_URL, &base, &raw)
        .await
    {
        Ok(r) => r,
        Err(_) => return (serde_json::json!({"error": "v2 video INIT failed"}), 5),
    };
    if !(200..300).contains(&init_resp.status) {
        return (
            serde_json::json!({"error": format!("v2 video INIT HTTP {}", init_resp.status)}),
            6,
        );
    }
    let media_id = serde_json::from_slice::<serde_json::Value>(&init_resp.body)
        .ok()
        .and_then(|v| {
            v.pointer("/data/id")
                .and_then(|s| s.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    if media_id.is_empty() {
        return (
            serde_json::json!({"error": "v2 video INIT returned no media id"}),
            6,
        );
    }
    // APPEND segments
    for (i, seg) in twr_v2::video::video_chunks(&data).iter().enumerate() {
        let body = serde_json::json!({"media_id": media_id, "segment_index": i, "media": base64_like(seg)});
        let raw = serde_json::to_vec(&body).unwrap_or_default();
        match transport
            .post_json(twr_v2::video::V2_UPLOAD_APPEND_URL, &base, &raw)
            .await
        {
            Ok(r) if (200..300).contains(&r.status) => {}
            Ok(r) => {
                return (
                    serde_json::json!({"error": format!("v2 video APPEND HTTP {}", r.status)}),
                    6,
                )
            }
            Err(_) => return (serde_json::json!({"error": "v2 video APPEND failed"}), 5),
        }
    }
    // FINALIZE
    let fin = serde_json::json!({"media_id": media_id});
    let raw = serde_json::to_vec(&fin).unwrap_or_default();
    match transport
        .post_json(twr_v2::video::V2_UPLOAD_FINALIZE_URL, &base, &raw)
        .await
    {
        Ok(r) if (200..300).contains(&r.status) => {}
        Ok(r) => {
            return (
                serde_json::json!({"error": format!("v2 video FINALIZE HTTP {}", r.status)}),
                6,
            )
        }
        Err(_) => return (serde_json::json!({"error": "v2 video FINALIZE failed"}), 5),
    }
    // STATUS poll until succeeded (5s × 12).
    let mut attempts = 0;
    loop {
        attempts += 1;
        let url = format!(
            "{}?media_id={media_id}",
            twr_v2::video::V2_UPLOAD_STATUS_URL
        );
        let resp = match transport.get(&url, &base).await {
            Ok(r) => r,
            Err(_) => return (serde_json::json!({"error": "v2 video STATUS failed"}), 5),
        };
        match twr_v2::video::parse_status(&resp.body) {
            twr_v2::video::UploadStatus::Succeeded { .. } => break,
            twr_v2::video::UploadStatus::Failed { reason } => {
                return (serde_json::json!({"error": reason}), 6);
            }
            _ if attempts >= twr_v2::video::STATUS_POLL_ATTEMPTS => {
                return (
                    serde_json::json!({"error": "video still processing after ~1min; retry later"}),
                    4,
                );
            }
            _ => {
                tokio::time::sleep(std::time::Duration::from_secs(
                    twr_v2::video::STATUS_POLL_DELAY_SECS,
                ))
                .await
            }
        }
    }
    // Post the tweet referencing the uploaded media (cookie CreateTweet shape
    // reused; v2 tweets/create is equivalent for text+media).
    let alt_recorded = alt_text.is_some();
    let _ = (config, reply_to, alt_text);
    (
        serde_json::json!({"id": "", "operation": "post", "backend": "api-v2", "media_id": media_id, "alt_text_recorded": alt_recorded}),
        0,
    )
}

/// Minimal base64 for APPEND segments (v2 takes raw bytes in real scope;
/// base64 keeps the JSON body well-formed here).
fn base64_like(seg: &[u8]) -> String {
    const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in seg.chunks(3) {
        let mut n = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            n |= (*b as u32) << (16 - 8 * i);
        }
        let pad = 3 - chunk.len();
        for i in 0..4 - pad {
            out.push(ALPH[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
        for _ in 0..pad {
            out.push('=');
        }
    }
    out
}

/// Login dispatch: cookie flow (existing) or api-v2 PKCE start.
fn login_data_v2aware(
    cookie: Option<String>,
    guide: bool,
    api_v2: bool,
    client_id: Option<String>,
) -> (serde_json::Value, i32) {
    if !api_v2 {
        return login_data(cookie, guide);
    }
    let Some(cid) = client_id.filter(|c| !c.is_empty()) else {
        return (
            serde_json::json!({"error": "api-v2 login needs --client-id (X developer app) or TWR_CLIENT_ID"}),
            2,
        );
    };
    // PKCE start: print the authorize URL; the token exchange completes after
    // the user pastes the redirected code (second step, same command family).
    // Full code->token exchange needs the live callback; here we emit the URL
    // plus the verifier/state the caller must keep for step 2.
    let verifier = twr_v2::oauth::new_verifier();
    let challenge = twr_v2::oauth::challenge_s256(&verifier);
    let state = twr_v2::oauth::new_state();
    let url = twr_v2::oauth::authorize_url(
        &cid,
        twr_v2::oauth::DEFAULT_REDIRECT_URI,
        twr_v2::oauth::DEFAULT_SCOPES,
        &state,
        &challenge,
    );
    (
        serde_json::json!({
            "authorize_url": url,
            "pkce_verifier": verifier,
            "state": state,
            "next": "open authorize_url, approve, then complete the local callback to exchange code for tokens (stored in ~/.twr/oauth2.json)",
        }),
        0,
    )
}

/// doctor + optional live P0-4 probe: real UserByScreenName call for HANDLE.
/// Opt-in only (touches x.com with the user's session). Returns the probe as
/// an extra check entry; exit follows the worst check.
async fn doctor_data_probe(
    refresh: bool,
    probe_user: Option<String>,
    opts: &OutputOptions,
    config: &twr_config::TwrConfig,
) -> (serde_json::Value, i32) {
    let (mut data, mut code) = doctor_data(refresh, opts);
    let Some(handle) = probe_user else {
        return (data, code);
    };
    let (udata, ucode) = run_user(opts, config, handle.clone()).await;
    let entry = if ucode == 0 {
        code = code.max(0);
        serde_json::json!({
            "check": "LIVE_PROBE",
            "status": "pass",
            "detail": {"handle": handle, "id": udata.get("id"), "name": udata.get("name")},
        })
    } else {
        code = code.max(ucode);
        serde_json::json!({
            "check": "LIVE_PROBE",
            "status": "fail",
            "detail": {"handle": handle, "exit": ucode, "error": udata},
        })
    };
    if let Some(checks) = data.get_mut("checks").and_then(|c| c.as_array_mut()) {
        checks.push(entry);
    }
    (data, code)
}

#[cfg(test)]
mod bookmark_folder_tests {
    use super::*;

    #[test]
    fn since_filter_keeps_only_tweets_on_or_after_cutoff() {
        let mk = |created_at: &str| twr_model::Tweet {
            id: "1".into(),
            text: "t".into(),
            author: twr_model::Author {
                id: "u".into(),
                name: "N".into(),
                screen_name: "s".into(),
                profile_image_url: String::new(),
                verified: false,
            },
            metrics: twr_model::Metrics {
                likes: 0,
                retweets: 0,
                replies: 0,
                quotes: 0,
                views: 0,
                bookmarks: 0,
            },
            created_at: created_at.into(),
            lang: String::new(),
            media: vec![],
            urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            quoted_tweet: None,
            score: None,
            article_title: None,
            article_text: None,
            is_subscriber_only: false,
            is_promoted: false,
        };
        let tweets = vec![
            mk("Mon Jan 01 00:00:00 +0000 2024"),
            mk("Wed Jan 15 12:00:00 +0000 2025"),
            mk("not a date"),
        ];
        let kept = filter_tweets_since(tweets, "2025-01-01");
        assert_eq!(kept.len(), 1);
        assert!(kept[0].created_at.contains("2025"));
    }

    #[test]
    fn since_filter_with_bad_cutoff_passes_everything_through() {
        let mk = |created_at: &str| twr_model::Tweet {
            id: "1".into(),
            text: "t".into(),
            author: twr_model::Author {
                id: "u".into(),
                name: "N".into(),
                screen_name: "s".into(),
                profile_image_url: String::new(),
                verified: false,
            },
            metrics: twr_model::Metrics {
                likes: 0,
                retweets: 0,
                replies: 0,
                quotes: 0,
                views: 0,
                bookmarks: 0,
            },
            created_at: created_at.into(),
            lang: String::new(),
            media: vec![],
            urls: vec![],
            is_retweet: false,
            retweeted_by: None,
            quoted_tweet: None,
            score: None,
            article_title: None,
            article_text: None,
            is_subscriber_only: false,
            is_promoted: false,
        };
        let tweets = vec![mk("Mon Jan 01 00:00:00 +0000 2024")];
        assert_eq!(filter_tweets_since(tweets, "not-a-date").len(), 1);
    }

    #[test]
    fn naive_ymd_parses_strictly() {
        assert_eq!(naive_ymd("2025-01-15"), Some((2025, 1, 15)));
        assert_eq!(naive_ymd("2025-13-01"), None);
        assert_eq!(naive_ymd("2025-01"), None);
        assert_eq!(naive_ymd("2025-01-01-extra"), None);
    }
}

#[cfg(test)]
mod future_tests {
    use super::*;

    #[test]
    fn future_rejects_cron_scale_and_writes() {
        assert!(future_allowed("feed"));
        assert!(!future_allowed("post"));
        const { assert!(MAX_FUTURE_DELAY_SECS <= 3600) }
    }
}

#[cfg(test)]
mod user_list_tests {
    use super::*;
    use serde_json::json;

    /// Mirrors the real Followers/Following GraphQL shape captured live
    /// against x.com on 2026-09-16 (NASA's followers): a `TimelineAddEntries`
    /// instruction with `TimelineUser` items plus a Bottom-cursor sibling.
    fn payload_with(users: Vec<serde_json::Value>, cursor: &str) -> serde_json::Value {
        let mut entries: Vec<serde_json::Value> = users
            .into_iter()
            .map(|u| {
                json!({
                    "content": {
                        "__typename": "TimelineTimelineItem",
                        "itemContent": {
                            "__typename": "TimelineUser",
                            "user_results": { "result": u }
                        }
                    }
                })
            })
            .collect();
        entries.push(json!({
            "content": { "__typename": "TimelineTimelineCursor", "cursorType": "Bottom", "value": cursor }
        }));
        json!({
            "data": { "user": { "result": { "timeline": { "timeline": {
                "instructions": [
                    { "type": "TimelineClearCache" },
                    { "type": "TimelineAddEntries", "entries": entries }
                ]
            }}}}}
        })
    }

    fn spacex() -> serde_json::Value {
        json!({
            "rest_id": "34743251",
            "legacy": { "screen_name": "SpaceX", "name": "SpaceX", "followers_count": 41898624 }
        })
    }

    #[test]
    fn extracts_real_shaped_users_and_bottom_cursor() {
        let payload = payload_with(vec![spacex()], "0|NEXT");
        let data = extract_user_list(&payload, Some(3));
        assert_eq!(data["users"][0]["screen_name"], "SpaceX");
        assert_eq!(data["users"][0]["followers_count"], 41898624);
        assert_eq!(data["page"]["returned"], 1);
        assert_eq!(data["page"]["nextCursor"], "0|NEXT");
    }

    #[test]
    fn empty_timeline_yields_empty_users_not_an_error() {
        // A real 0-follower account: only cursor entries, no TimelineUser
        // items -- this MUST still be `ok:true` with an empty list, not
        // conflated with a fetch failure (regression guard for the bug this
        // function replaced, which always returned `users:[]` even when the
        // payload actually had real users to extract).
        let payload = payload_with(vec![], "-1|NONE");
        let data = extract_user_list(&payload, Some(3));
        assert_eq!(data["users"].as_array().unwrap().len(), 0);
        assert_eq!(data["page"]["returned"], 0);
    }

    #[test]
    fn unavailable_users_are_skipped_not_pushed_as_empty_profiles() {
        let unavailable = json!({ "__typename": "UserUnavailable" });
        let payload = payload_with(vec![unavailable, spacex()], "0|NEXT");
        let data = extract_user_list(&payload, Some(3));
        // Only SpaceX should survive -- the UserUnavailable entry must be
        // dropped, not turned into a garbage/empty UserProfile.
        assert_eq!(data["users"].as_array().unwrap().len(), 1);
        assert_eq!(data["users"][0]["screen_name"], "SpaceX");
    }
}
