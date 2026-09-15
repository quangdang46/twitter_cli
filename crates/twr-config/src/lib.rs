//! `twr-config` — figment-based config resolution (plan §7).
//!
//! Order: `./config.yaml` (cwd) → `~/.twr/config.yaml` → built-in defaults,
//! deep-merged, with `TWITTER_*` env overrides on top.
//!
//! Two deliberate divergences from the Python original:
//! - Paths resolve strictly cwd-then-home, NEVER relative to the binary's own
//!   install location (fixes upstream issue #50's second bug, where
//!   `parent.parent` landed inside `site-packages` under uv). There is a
//!   regression test pinning this.
//! - `masked_view()` redacts secret-shaped values before anything prints them
//!   (plan §0.1 #8).

use figment::{providers::Env, Figment};
use serde::{Deserialize, Serialize};

/// Built-in defaults, ported exactly from plan §1.2 Config.
pub fn default_config() -> TwrConfig {
    TwrConfig {
        fetch: FetchConfig { count: 50 },
        filter: FilterConfig {
            mode: "topN".to_string(),
            top_n: 20,
            max: 50,
            weights: FilterWeights {
                likes: 1.0,
                retweets: 3.0,
                replies: 2.0,
                bookmarks: 5.0,
                views_log: 0.5,
            },
        },
        rate_limit: RateLimitConfig {
            request_delay_secs: 1.5,
            retries: 3,
            backoff_secs: 5.0,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TwrConfig {
    #[serde(default)]
    pub fetch: FetchConfig,
    #[serde(default)]
    pub filter: FilterConfig,
    #[serde(default, rename = "rateLimit")]
    pub rate_limit: RateLimitConfig,
}

impl Default for TwrConfig {
    fn default() -> Self {
        default_config()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetchConfig {
    pub count: u32,
}

impl Default for FetchConfig {
    fn default() -> Self {
        default_config().fetch
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterConfig {
    #[serde(default = "default_filter_mode")]
    pub mode: String,
    #[serde(default = "default_top_n")]
    pub top_n: u32,
    #[serde(default = "default_filter_max")]
    pub max: u32,
    #[serde(default)]
    pub weights: FilterWeights,
}

fn default_filter_mode() -> String {
    "topN".to_string()
}
fn default_top_n() -> u32 {
    20
}
fn default_filter_max() -> u32 {
    50
}

impl Default for FilterConfig {
    fn default() -> Self {
        default_config().filter
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterWeights {
    pub likes: f64,
    pub retweets: f64,
    pub replies: f64,
    pub bookmarks: f64,
    pub views_log: f64,
}

impl Default for FilterWeights {
    fn default() -> Self {
        default_config().filter.weights
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default = "default_request_delay", rename = "requestDelay")]
    pub request_delay_secs: f64,
    #[serde(default = "default_retries")]
    pub retries: u32,
    #[serde(default = "default_backoff", rename = "backoff")]
    pub backoff_secs: f64,
}

fn default_request_delay() -> f64 {
    1.5
}
fn default_retries() -> u32 {
    3
}
fn default_backoff() -> f64 {
    5.0
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        default_config().rate_limit
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not parse {path}: {detail}")]
    Parse { path: String, detail: String },
}

/// Candidate config paths in resolution order. Strictly cwd-then-home —
/// never relative to the running binary (issue #50 regression pin).
pub fn candidate_paths(
    cwd: &std::path::Path,
    home: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut paths = vec![cwd.join("config.yaml")];
    if let Some(home) = home {
        paths.push(home.join(".twr").join("config.yaml"));
    }
    paths
}

/// Load + merge: each existing file (cwd first, then home) deep-merges over
/// the previous layer via figment, then `TWITTER_*` env overrides apply.
/// Missing/unparseable files are skipped (parse errors reported, not fatal —
/// a broken optional file must not brick the CLI).
pub fn load(
    cwd: &std::path::Path,
    home: Option<&std::path::Path>,
) -> (TwrConfig, Vec<ConfigError>) {
    let mut errors = Vec::new();
    let mut figment = Figment::from(figment::providers::Serialized::defaults(default_config()));

    for path in candidate_paths(cwd, home) {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_yaml::from_str::<serde_json::Value>(&raw) {
            Ok(value) => {
                figment = figment.merge(figment::providers::Serialized::defaults(value));
            }
            Err(e) => errors.push(ConfigError::Parse {
                path: path.display().to_string(),
                detail: e.to_string(),
            }),
        }
    }

    figment = figment.merge(Env::prefixed("TWITTER_").split("__"));

    match figment.extract::<TwrConfig>() {
        Ok(config) => (config, errors),
        Err(e) => {
            errors.push(ConfigError::Parse {
                path: "<env>".to_string(),
                detail: e.to_string(),
            });
            (default_config(), errors)
        }
    }
}

/// Keys whose values must be masked in `config show` output.
pub const SECRET_KEY_SUFFIXES: &[&str] = &["token", "cookie", "ct0", "secret", "proxy", "key"];

pub fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SECRET_KEY_SUFFIXES.iter().any(|s| lower.contains(s))
}

/// Serialize the config with secret-shaped values replaced by `[REDACTED]`.
/// Walks the JSON value tree so nested secrets are masked too.
pub fn masked_view(config: &TwrConfig) -> serde_json::Value {
    mask(serde_json::to_value(config).unwrap_or_default())
}

fn mask(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    if is_secret_key(&k) && v.is_string() {
                        (k, serde_json::Value::String("[REDACTED]".to_string()))
                    } else {
                        (k, mask(v))
                    }
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(mask).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_match_plan_section_1_2() {
        let c = default_config();
        assert_eq!(c.fetch.count, 50);
        assert_eq!(c.filter.mode, "topN");
        assert_eq!(c.filter.top_n, 20);
        assert_eq!(c.filter.max, 50);
        assert_eq!(c.filter.weights.likes, 1.0);
        assert_eq!(c.filter.weights.retweets, 3.0);
        assert_eq!(c.filter.weights.replies, 2.0);
        assert_eq!(c.filter.weights.bookmarks, 5.0);
        assert_eq!(c.filter.weights.views_log, 0.5);
        assert_eq!(c.rate_limit.request_delay_secs, 1.5);
        assert_eq!(c.rate_limit.retries, 3);
        assert_eq!(c.rate_limit.backoff_secs, 5.0);
    }

    #[test]
    fn resolution_is_cwd_then_home_never_binary_dir() {
        // Issue #50 regression pin: candidate paths derive from the passed
        // cwd + home only. The binary's own dir must not appear.
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap();
        let cwd = std::path::Path::new("/tmp/some-cwd");
        let home = std::path::Path::new("/tmp/some-home");
        let paths = candidate_paths(cwd, Some(home));
        assert_eq!(
            paths,
            vec![
                cwd.join("config.yaml"),
                home.join(".twr").join("config.yaml")
            ]
        );
        assert!(
            !paths.iter().any(|p| p.starts_with(&exe_dir)),
            "config resolution must never touch the install dir {exe_dir:?}"
        );
    }

    #[test]
    fn missing_files_yield_defaults() {
        let cwd = std::path::Path::new("/tmp/twr-config-definitely-missing-1");
        let (c, errors) = load(cwd, None);
        assert_eq!(c, default_config());
        assert!(errors.is_empty());
    }

    #[test]
    fn cwd_file_overrides_defaults_and_home_overrides_cwd() {
        let base = std::env::temp_dir().join(format!("twr-config-{}", std::process::id()));
        let cwd = base.join("cwd");
        let home = base.join("home");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(home.join(".twr")).unwrap();
        std::fs::write(cwd.join("config.yaml"), "fetch:\n  count: 10\n").unwrap();
        std::fs::write(
            home.join(".twr").join("config.yaml"),
            "fetch:\n  count: 25\nfilter:\n  top_n: 7\n",
        )
        .unwrap();
        let (c, errors) = load(&cwd, Some(&home));
        assert!(errors.is_empty());
        assert_eq!(c.fetch.count, 25);
        assert_eq!(c.filter.top_n, 7);
        // Untouched keys keep defaults (deep merge, not replace).
        assert_eq!(c.filter.weights.likes, 1.0);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn broken_file_is_reported_not_fatal() {
        let base = std::env::temp_dir().join(format!("twr-config-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("config.yaml"), "fetch:\n  count: [unclosed\n").unwrap();
        let (c, errors) = load(&base, None);
        assert_eq!(c.fetch.count, 50);
        assert_eq!(errors.len(), 1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn masked_view_redacts_secret_shaped_values() {
        assert!(is_secret_key("auth_token"));
        assert!(is_secret_key("ct0"));
        assert!(is_secret_key("TWITTER_PROXY"));
        assert!(!is_secret_key("count"));
        // No live secrets exist in TwrConfig today; assert the walk redacts
        // them if the struct ever gains one by masking a synthetic tree.
        let raw = json!({
            "fetch": {"count": 50},
            "auth_token": "live-secret",
            "nested": {"ct0": "live-secret", "count": 3},
        });
        let masked = masked_view(&default_config());
        let s = serde_json::to_string(&masked).unwrap();
        assert!(!s.contains("live-secret"));
        let masked_raw = mask(raw);
        assert_eq!(masked_raw["auth_token"], json!("[REDACTED]"));
        assert_eq!(masked_raw["nested"]["ct0"], json!("[REDACTED]"));
        assert_eq!(masked_raw["nested"]["count"], json!(3));
    }
}
