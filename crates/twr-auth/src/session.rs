//! Session-file persistence: `~/.twr/session.json`.
//!
//! The file holds the raw `auth_token` + `ct0` pair (it MUST be readable only
//! by the owner — created with `0o600` on unix). Diagnostics never print its
//! contents; see [`SessionStatus`] for the redacted view.

use crate::SessionCookies;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default session-file location: `~/.twr/session.json`.
pub fn default_session_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".twr").join("session.json"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionFile {
    auth_token: Option<String>,
    ct0: Option<String>,
}

/// Load a session from disk. `None` = missing/unparseable/incomplete — never
/// an error the caller must handle, since every case just falls through to
/// the next auth layer.
pub fn load(path: &Path) -> Option<SessionCookies> {
    let raw = std::fs::read_to_string(path).ok()?;
    let file: SessionFile = serde_json::from_str(&raw).ok()?;
    let session = SessionCookies {
        auth_token: file.auth_token.filter(|v| !v.is_empty()),
        ct0: file.ct0.filter(|v| !v.is_empty()),
    };
    if session.is_complete() {
        Some(session)
    } else {
        None
    }
}

/// Save a session, creating parent dirs and locking the file to owner-only
/// (`0o600`) on unix. Refuses to overwrite a *still-valid* session unless
/// `force` — the agent-x pattern: `logout` first, then `login`.
/// Returns `AlreadyValid` when refusing, so callers can suggest `logout`.
#[derive(Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    AlreadyValid,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SaveError {
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "could not write session file: {e}"),
        }
    }
}

pub fn save(path: &Path, session: &SessionCookies, force: bool) -> Result<SaveOutcome, SaveError> {
    if !force && load(path).is_some() {
        return Ok(SaveOutcome::AlreadyValid);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| SaveError::Io(e.to_string()))?;
    }
    let file = SessionFile {
        auth_token: session.auth_token.clone(),
        ct0: session.ct0.clone(),
    };
    let raw = serde_json::to_string(&file).map_err(|e| SaveError::Io(e.to_string()))?;
    std::fs::write(path, raw).map_err(|e| SaveError::Io(e.to_string()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perm = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perm);
    }

    Ok(SaveOutcome::Saved)
}

/// Delete the session file. Missing file = success (idempotent logout).
pub fn clear(path: &Path) -> Result<(), SaveError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SaveError::Io(e.to_string())),
    }
}

/// Redacted session status — the ONLY view diagnostics may print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStatus {
    pub present: bool,
    pub source: Option<&'static str>,
}

impl SessionStatus {
    pub fn absent() -> Self {
        Self {
            present: false,
            source: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("twr-auth-session-{}-{}", tag, std::process::id()))
    }

    fn complete() -> SessionCookies {
        SessionCookies {
            auth_token: Some("tok".into()),
            ct0: Some("ct".into()),
        }
    }

    #[test]
    fn save_then_load_round_trip() {
        let dir = tmp_path("rt");
        let path = dir.join("session.json");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(save(&path, &complete(), false).unwrap(), SaveOutcome::Saved);
        let loaded = load(&path).unwrap();
        assert!(loaded.is_complete());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_refuses_to_overwrite_valid_session_without_force() {
        let dir = tmp_path("refuse");
        let path = dir.join("session.json");
        let _ = std::fs::remove_dir_all(&dir);
        save(&path, &complete(), false).unwrap();
        assert_eq!(
            save(&path, &complete(), false).unwrap(),
            SaveOutcome::AlreadyValid
        );
        assert_eq!(save(&path, &complete(), true).unwrap(), SaveOutcome::Saved);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rejects_missing_garbage_and_incomplete() {
        let dir = tmp_path("reject");
        let path = dir.join("session.json");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load(&path).is_none());
        std::fs::write(&path, "not json{{").unwrap();
        assert!(load(&path).is_none());
        std::fs::write(&path, r#"{"auth_token":"only"}"#).unwrap();
        assert!(load(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_is_idempotent() {
        let dir = tmp_path("clear");
        let path = dir.join("session.json");
        let _ = std::fs::remove_dir_all(&dir);
        clear(&path).unwrap();
        save(&path, &complete(), false).unwrap();
        clear(&path).unwrap();
        assert!(load(&path).is_none());
        clear(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn session_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp_path("perm");
        let path = dir.join("session.json");
        let _ = std::fs::remove_dir_all(&dir);
        save(&path, &complete(), false).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "session file must be owner-only");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
