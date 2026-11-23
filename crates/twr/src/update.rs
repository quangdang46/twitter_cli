//! `twr update` + the passive once-per-24h version check (issue #25).
//!
//! Both paths talk to the GitHub Releases API only — never x.com — so this
//! is NOT an X write: no `--policy write`, no auth/session state, no
//! mutation budgets. The passive check prints ONE stderr line on version
//! mismatch (never inside the JSON envelope), is silent when up-to-date,
//! offline, or rate-limited, and is throttled by `<state_dir>/update-check.json`.
//! Opt out with `TWR_NO_UPDATE_CHECK=1` or `TWR_UPDATE_CHECK=0`.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use twr_client::HttpTransport;

const API_BASE: &str = "https://api.github.com/repos/quangdang46/twitter_cli";
const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

fn installed_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `Some(true)`/`Some(false)` for a boolean-ish env var; `None` when unset
/// or empty. Values in {"0","false","no","off"} are false; everything else
/// non-empty is true.
fn env_flag(name: &str) -> Option<bool> {
    match std::env::var(name) {
        Ok(v) => {
            let v = v.trim();
            if v.is_empty() {
                None
            } else {
                Some(!matches!(
                    v.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                ))
            }
        }
        Err(_) => None,
    }
}

/// Passive check opt-out (issue #25): `TWR_NO_UPDATE_CHECK=1`, or
/// `TWR_UPDATE_CHECK=0` — for CI and air-gapped environments.
pub fn check_disabled() -> bool {
    env_flag("TWR_NO_UPDATE_CHECK") == Some(true) || env_flag("TWR_UPDATE_CHECK") == Some(false)
}

/// `v0.1.7` / `0.1.7` → `(0,1,7)`. Strict: exactly three numeric parts.
fn version_parts(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let mut it = v.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

fn newer(remote: &str, local: &str) -> bool {
    match (version_parts(remote), version_parts(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

fn same_version(a: &str, b: &str) -> bool {
    match (version_parts(a), version_parts(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim().trim_start_matches('v') == b.trim().trim_start_matches('v'),
    }
}

/// Release asset suffix for this platform (mirrors install.sh's
/// `platform_to_suffix`). `None` = no prebuilt asset for this triple.
fn platform_suffix() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        _ => None,
    }
}

fn asset_extension() -> &'static str {
    if cfg!(windows) {
        "zip"
    } else {
        "tar.gz"
    }
}

fn exe_file_name() -> &'static str {
    if cfg!(windows) {
        "twr.exe"
    } else {
        "twr"
    }
}

#[derive(Debug)]
struct Release {
    /// Version without the leading `v` (e.g. "0.1.7").
    version: String,
    /// `(asset name, download URL)` pairs.
    assets: Vec<(String, String)>,
}

fn parse_release(body: &[u8]) -> Result<Release, String> {
    let v: Value =
        serde_json::from_slice(body).map_err(|e| format!("unparseable release JSON: {e}"))?;
    let tag = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "release JSON has no tag_name".to_string())?;
    let version = tag.trim_start_matches('v').to_string();
    let mut assets = Vec::new();
    if let Some(list) = v.get("assets").and_then(|a| a.as_array()) {
        for a in list {
            if let (Some(name), Some(url)) = (
                a.get("name").and_then(|n| n.as_str()),
                a.get("browser_download_url").and_then(|u| u.as_str()),
            ) {
                assets.push((name.to_string(), url.to_string()));
            }
        }
    }
    Ok(Release { version, assets })
}

async fn fetch_release(transport: &dyn HttpTransport, url: &str) -> Result<Release, String> {
    let ua = format!("twr/{}", installed_version());
    let resp = transport
        .get(
            url,
            &[
                ("Accept", "application/vnd.github+json"),
                ("User-Agent", ua.as_str()),
            ],
        )
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?;
    if resp.status != 200 {
        return Err(format!("GitHub API HTTP {}", resp.status));
    }
    parse_release(&resp.body)
}

async fn fetch_latest_version(transport: &dyn HttpTransport) -> Option<String> {
    let url = format!("{API_BASE}/releases/latest");
    fetch_release(transport, &url).await.ok().map(|r| r.version)
}

/// Pick the archive asset for this platform out of a release's asset list.
fn select_asset<'a>(rel: &'a Release, suffix: &str) -> Result<&'a (String, String), String> {
    let want = format!("twr-{suffix}.{}", asset_extension());
    rel.assets
        .iter()
        .find(|(name, _)| name == &want)
        .ok_or_else(|| {
            format!(
                "release {} has no asset {want} (supported: {})",
                rel.version,
                rel.assets
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// First whitespace-separated token of a `.sha256` sidecar
/// (`<hex>  <filename>` or bare `<hex>`).
fn parse_sha256_sidecar(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let token = text.split_whitespace().next()?;
    (token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| token.to_ascii_lowercase())
}

async fn download_bytes(transport: &dyn HttpTransport, url: &str) -> Result<Vec<u8>, String> {
    let ua = format!("twr/{}", installed_version());
    let resp = transport
        .get(url, &[("User-Agent", ua.as_str())])
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if resp.status != 200 {
        return Err(format!("download HTTP {} for {url}", resp.status));
    }
    Ok(resp.body)
}

fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("twr-{tag}-{}", std::process::id()))
}

/// Extract the `twr`/`twr.exe` binary out of a release archive (tar.gz on
/// unix, zip on Windows) into memory.
fn extract_binary(archive: &[u8], asset_name: &str) -> Result<Vec<u8>, String> {
    let dir = temp_dir("update-x");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let result = extract_into(archive, asset_name, &dir).and_then(|()| {
        find_binary(&dir, exe_file_name())
            .ok_or_else(|| "archive has no twr binary".to_string())
            .and_then(|p| std::fs::read(&p).map_err(|e| format!("read extracted binary: {e}")))
    });
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn extract_into(archive: &[u8], asset_name: &str, dir: &Path) -> Result<(), String> {
    let arc_path = dir.join("archive.bin");
    std::fs::write(&arc_path, archive).map_err(|e| format!("stage archive: {e}"))?;
    let file = std::fs::File::open(&arc_path).map_err(|e| format!("open archive: {e}"))?;
    let out_dir = dir.join("x");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("extract dir: {e}"))?;
    if asset_name.ends_with(".tar.gz") {
        let gz = flate2::read::GzDecoder::new(file);
        let mut ar = tar::Archive::new(gz);
        ar.unpack(&out_dir)
            .map_err(|e| format!("tar extract: {e}"))?;
    } else {
        extract_zip(file, out_dir)?;
    }
    Ok(())
}

#[cfg(windows)]
fn extract_zip(file: std::fs::File, dir: PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&dir).map_err(|e| format!("extract dir: {e}"))?;
    let mut z = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| format!("zip open: {e}"))?;
    z.extract(&dir).map_err(|e| format!("zip extract: {e}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn extract_zip(_file: std::fs::File, _dir: PathBuf) -> Result<(), String> {
    Err("zip extraction is only supported on Windows".into())
}

/// Depth-first search for the first file named `name` under `dir`.
fn find_binary(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).ok()?;
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
                return Some(p);
            }
        }
    }
    None
}

/// First `twr` executable on PATH that is NOT the running binary — the
/// PATH-shadow case where an update would not take effect. `path_env` is
/// injectable for tests.
fn path_shadow(exe: &Path, path_env: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let path_env = path_env?;
    let exe_name = exe_file_name();
    let canon_exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    for dir in std::env::split_paths(path_env) {
        let cand = dir.join(exe_name);
        if !is_executable(&cand) {
            continue;
        }
        let canon = cand.canonicalize().unwrap_or_else(|_| cand.clone());
        return (canon != canon_exe).then_some(cand);
    }
    None
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// True when the directory containing `dest` accepts new files (the actual
/// replaceability test — on unix a running exe can't be opened O_WRONLY
/// but CAN be renamed over; on Windows we never rename the live exe).
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".twr-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// `<state_dir>/update-check.json` cache for the passive 24h check.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct CheckCache {
    #[serde(default)]
    checked_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notified: Option<String>,
}

fn check_cache_path() -> Option<PathBuf> {
    twr_core::budget::state_dir().map(|d| d.join("update-check.json"))
}

fn load_check_cache(path: &Path) -> CheckCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_check_cache(path: &Path, cache: &CheckCache) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, s);
    }
}

/// Passive version check: at most one GitHub API hit per 24h per state
/// dir; prints the single stderr notice on mismatch. Every failure mode
/// (offline, rate-limited, no state dir) is silent. Bounded by
/// `CHECK_TIMEOUT` so it can never stall a command meaningfully.
pub async fn maybe_notify_update(quiet: bool) {
    if quiet || check_disabled() {
        return;
    }
    let Some(path) = check_cache_path() else {
        return;
    };
    let now = unix_now();
    let mut cache = load_check_cache(&path);
    let fresh = cache.checked_at > 0 && now.saturating_sub(cache.checked_at) < CHECK_INTERVAL_SECS;

    if !fresh {
        let fetched = match twr_client::WreqTransport::new_chrome_with_timeout(CHECK_TIMEOUT) {
            Ok(t) => tokio::time::timeout(
                CHECK_TIMEOUT + Duration::from_secs(2),
                fetch_latest_version(&t),
            )
            .await
            .ok()
            .flatten(),
            Err(_) => None,
        };
        // Record the attempt even on failure: an offline/air-gapped host
        // must not pay the timeout on EVERY invocation.
        cache.checked_at = now;
        if let Some(v) = fetched {
            cache.latest = Some(v);
        }
        save_check_cache(&path, &cache);
    }

    let Some(latest) = cache.latest.clone() else {
        return;
    };
    let installed = installed_version();
    if !newer(&latest, installed) {
        return;
    }
    if cache.notified.as_deref() == Some(latest.as_str()) {
        return;
    }
    eprintln!("twr {latest} available (installed {installed}) — run `twr update`");
    cache.notified = Some(latest);
    save_check_cache(&path, &cache);
}

/// `twr update [--check] [--version x.y.z]` — returns `(data, exit_code)`
/// like every other handler; the caller wraps it in the envelope.
pub async fn run_update(check_only: bool, pin: Option<String>) -> (Value, i32) {
    match update_inner(check_only, pin).await {
        Ok(data) => (data, 0),
        Err((msg, code)) => (json!({"error": msg}), code),
    }
}

async fn update_inner(check_only: bool, pin: Option<String>) -> Result<Value, (String, i32)> {
    let Some(suffix) = platform_suffix() else {
        return Err((
            format!(
                "no prebuilt release asset for {}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
            2,
        ));
    };

    // Resolve the pinned version tag, if any.
    let pinned_version = match pin.as_deref() {
        Some(p) => {
            let v = p.trim().trim_start_matches('v');
            match version_parts(v) {
                Some(_) => Some(v.to_string()),
                None => return Err((format!("invalid --version {p:?} (expected x.y.z)"), 2)),
            }
        }
        None => None,
    };

    let transport = twr_client::WreqTransport::new_chrome_with_timeout(DOWNLOAD_TIMEOUT)
        .map_err(|e| (format!("transport: {e}"), 5))?;

    let rel = match &pinned_version {
        Some(v) => {
            let url = format!("{API_BASE}/releases/tags/v{v}");
            fetch_release(&transport, &url).await.map_err(|e| {
                let code = if e.contains("HTTP 404") { 3 } else { 5 };
                (e, code)
            })?
        }
        None => fetch_release(&transport, &format!("{API_BASE}/releases/latest"))
            .await
            .map_err(|e| (e, 5))?,
    };

    let (_, asset_url) = select_asset(&rel, suffix).map_err(|e| (e, 3))?;
    let installed = installed_version();

    if check_only {
        return Ok(json!({
            "check": true,
            "installed": installed,
            "latest": rel.version,
            "update_available": !same_version(&rel.version, installed),
            "asset": format!("twr-{suffix}.{}", asset_extension()),
        }));
    }

    if same_version(&rel.version, installed) {
        return Ok(json!({
            "updated": false,
            "reason": "already up to date",
            "installed": installed,
            "latest": rel.version,
        }));
    }

    // Download archive + sha256 sidecar; verify before touching anything.
    let archive_name = format!("twr-{suffix}.{}", asset_extension());
    let archive = download_bytes(&transport, asset_url)
        .await
        .map_err(|e| (e, 5))?;
    let sha_url = format!("{asset_url}.sha256");
    let sha_body = download_bytes(&transport, &sha_url)
        .await
        .map_err(|e| (format!("checksum sidecar unavailable: {e}"), 5))?;
    let expected = parse_sha256_sidecar(&sha_body)
        .ok_or_else(|| ("malformed .sha256 sidecar".to_string(), 6))?;
    let actual = sha256_hex(&archive);
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err((
            format!("sha256 mismatch for {archive_name}: expected {expected}, got {actual}"),
            6,
        ));
    }

    let binary = extract_binary(&archive, &archive_name).map_err(|e| (e, 6))?;

    // Locate + replace the running binary.
    let exe = std::env::current_exe().map_err(|e| (format!("current_exe: {e}"), 2))?;
    let exe_name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if exe_name != exe_file_name() {
        return Err((
            format!(
                "refusing to self-update: current exe is {exe_name:?}, expected {:?}",
                exe_file_name()
            ),
            2,
        ));
    }
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    if !dir_writable(dir) {
        return Err((format!("install dir {} is not writable", dir.display()), 2));
    }

    let shadow = path_shadow(&exe, std::env::var_os("PATH").as_deref());

    #[cfg(windows)]
    {
        // A running .exe cannot be overwritten or renamed on Windows:
        // stage twr.new.exe next to it and hand the user the rename step.
        let staged = dir.join("twr.new.exe");
        std::fs::write(&staged, &binary)
            .map_err(|e| (format!("write {}: {e}", staged.display()), 2))?;
        let mut data = json!({
            "updated": false,
            "staged": staged.display().to_string(),
            "from": installed,
            "to": rel.version,
            "manual_step": format!("close all twr processes, then run: move /Y \"{}\" \"{}\"", staged.display(), exe.display()),
        });
        if let Some(s) = &shadow {
            data["path_shadow"] = json!({"shadowing": s.display().to_string()});
        }
        Ok(data)
    }

    #[cfg(not(windows))]
    {
        let tmp = dir.join(format!(".twr.new-{}", std::process::id()));
        std::fs::write(&tmp, &binary).map_err(|e| (format!("write {}: {e}", tmp.display()), 2))?;
        // Preserve the existing mode bits where possible (unix; on
        // Windows set_permissions only toggles the readonly flag, which
        // we don't want copied from the live exe).
        #[cfg(unix)]
        if let Ok(meta) = std::fs::metadata(&exe) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        if let Err(e) = std::fs::rename(&tmp, &exe) {
            let _ = std::fs::remove_file(&tmp);
            return Err((format!("replace {}: {e}", exe.display()), 2));
        }
        let mut data = json!({
            "updated": true,
            "installed": installed,
            "latest": rel.version,
            "path": exe.display().to_string(),
        });
        if let Some(s) = &shadow {
            data["path_shadow"] = json!({"shadowing": s.display().to_string()});
            eprintln!(
                "twr warning: {} earlier on PATH shadows the updated binary — remove it or reorder PATH",
                s.display()
            );
        }
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FakeTransport {
        status: u16,
        body: Vec<u8>,
        seen: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            url: &str,
            _headers: &[(&str, &str)],
        ) -> Result<twr_client::TransportResponse, twr_client::TransportError> {
            self.seen.lock().unwrap().push(url.to_string());
            Ok(twr_client::TransportResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
        async fn post_json(
            &self,
            _url: &str,
            _headers: &[(&str, &str)],
            _body: &[u8],
        ) -> Result<twr_client::TransportResponse, twr_client::TransportError> {
            unreachable!()
        }
    }

    fn release_json(tag: &str, assets: &[&str]) -> Vec<u8> {
        let assets: Vec<Value> = assets
            .iter()
            .map(|n| json!({"name": n, "browser_download_url": format!("https://example/{n}")}))
            .collect();
        serde_json::to_vec(&json!({"tag_name": tag, "assets": assets})).unwrap()
    }

    #[test]
    fn version_compare() {
        assert!(newer("v0.1.7", "0.1.6"));
        assert!(!newer("0.1.6", "0.1.6"));
        assert!(!newer("0.1.5", "0.1.6"));
        assert!(newer("0.2.0", "0.1.9"));
        assert!(same_version("v0.1.6", "0.1.6"));
        assert!(!same_version("v0.1.7", "0.1.6"));
        assert!(version_parts("v1.2.3").is_some());
        assert!(version_parts("1.2").is_none());
        assert!(version_parts("1.2.3.4").is_none());
    }

    #[test]
    fn asset_selection_matches_platform() {
        let suffix = platform_suffix().unwrap();
        let rel = parse_release(&release_json(
            "v9.9.9",
            &[
                "twr-linux-x86_64.tar.gz",
                "twr-macos-aarch64.tar.gz",
                "twr-macos-x86_64.tar.gz",
                "twr-windows-x86_64.zip",
            ],
        ))
        .unwrap();
        let want = format!("twr-{suffix}.{}", asset_extension());
        assert!(select_asset(&rel, suffix).is_ok(), "missing {want}");
        // A suffix with no matching asset errors with the asset list.
        let err = select_asset(&rel, "plan9-mips").unwrap_err();
        assert!(err.contains("plan9-mips") || err.contains("twr-plan9"));
    }

    #[tokio::test]
    async fn fetch_release_parses_tag_and_assets() {
        let t = FakeTransport {
            status: 200,
            body: release_json("v0.1.7", &["twr-linux-x86_64.tar.gz"]),
            seen: Mutex::new(vec![]),
        };
        let rel = fetch_release(&t, "https://api.example/release")
            .await
            .unwrap();
        assert_eq!(rel.version, "0.1.7");
        assert_eq!(rel.assets.len(), 1);
        assert_eq!(t.seen.lock().unwrap()[0], "https://api.example/release");
    }

    #[tokio::test]
    async fn fetch_release_non_200_is_error() {
        let t = FakeTransport {
            status: 403,
            body: vec![],
            seen: Mutex::new(vec![]),
        };
        let err = fetch_release(&t, "https://api.example/release")
            .await
            .unwrap_err();
        assert!(err.contains("403"));
    }

    #[test]
    fn sha256_sidecar_parsing_and_verify() {
        let bytes = b"hello";
        let sum = sha256_hex(bytes);
        assert_eq!(sum.len(), 64);
        let sidecar = format!("{sum}  twr-linux-x86_64.tar.gz\n");
        assert_eq!(parse_sha256_sidecar(sidecar.as_bytes()), Some(sum.clone()));
        // Bare hex also parses; garbage does not.
        assert_eq!(parse_sha256_sidecar(sum.as_bytes()), Some(sum));
        assert_eq!(parse_sha256_sidecar(b"not-a-hash"), None);
    }

    #[test]
    fn tar_gz_roundtrip_extracts_binary() {
        // Build a tar.gz in memory containing twr at a nested path.
        let mut tar_buf = Vec::new();
        {
            let enc = flate2::write::GzEncoder::new(&mut tar_buf, flate2::Compression::fast());
            let mut builder = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            let payload = b"#!/bin/sh\necho twr\n";
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "dist/twr", &payload[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let out = extract_binary(&tar_buf, "twr-linux-x86_64.tar.gz").unwrap();
        assert_eq!(out, b"#!/bin/sh\necho twr\n");
    }

    #[test]
    fn path_shadow_detects_earlier_path_hit() {
        let dir = std::env::temp_dir().join(format!("twr-shadow-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("twr");
        std::fs::write(&fake, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // PATH containing only the fake dir → shadow (fake != current exe).
        let shadow = path_shadow(
            &std::env::current_exe().unwrap(),
            Some(std::ffi::OsStr::new(dir.to_str().unwrap())),
        );
        assert_eq!(shadow, Some(fake.clone()));
        // A symlink named `twr` pointing at the running exe counts as the
        // same binary (canonicalize resolves it).
        let same_dir = dir.join("same");
        std::fs::create_dir_all(&same_dir).unwrap();
        let link = same_dir.join("twr");
        #[cfg(unix)]
        std::os::unix::fs::symlink(std::env::current_exe().unwrap(), &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(std::env::current_exe().unwrap(), &link).unwrap();
        // PATH where the running exe's own entry comes first → no shadow.
        let both = format!("{}:{}", same_dir.display(), dir.display());
        assert!(path_shadow(
            &std::env::current_exe().unwrap(),
            Some(std::ffi::OsStr::new(&both))
        )
        .is_none());
        // Reversed order → the fake earlier copy shadows.
        let reversed = format!("{}:{}", dir.display(), same_dir.display());
        assert_eq!(
            path_shadow(
                &std::env::current_exe().unwrap(),
                Some(std::ffi::OsStr::new(&reversed))
            ),
            Some(fake.clone())
        );
        // Empty PATH → none.
        assert!(path_shadow(
            &std::env::current_exe().unwrap(),
            Some(std::ffi::OsStr::new(""))
        )
        .is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_opt_out_flags() {
        // These are process-global; only assert the pure mapping shape by
        // calling env_flag on a var we control.
        // (TWR_* vars may be set in the environment; don't mutate them in
        // tests that run in parallel — assert the parser via a temp var.)
        std::env::set_var("TWR_TEST_FLAG_A", "1");
        std::env::set_var("TWR_TEST_FLAG_B", "0");
        std::env::set_var("TWR_TEST_FLAG_C", "");
        assert_eq!(env_flag("TWR_TEST_FLAG_A"), Some(true));
        assert_eq!(env_flag("TWR_TEST_FLAG_B"), Some(false));
        assert_eq!(env_flag("TWR_TEST_FLAG_C"), None);
        assert_eq!(env_flag("TWR_TEST_FLAG_UNSET_XYZ"), None);
        std::env::remove_var("TWR_TEST_FLAG_A");
        std::env::remove_var("TWR_TEST_FLAG_B");
        std::env::remove_var("TWR_TEST_FLAG_C");
    }
}
