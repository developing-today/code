//! Session identity inherited by Fleet commands and recovered per worktree.
//!
//! `.fleet-session-id` is deliberately a plain, one-line file at the Git
//! worktree root. It is not shared through the common Git directory, so each
//! worktree retains its own active session association.

use std::path::{Path, PathBuf};

pub const PWD_FILE: &str = ".fleet-session-id";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Session(String),
    None,
}

/// Resolve an explicit CLI value, then the Fleet/OpenCode environment, then
/// the worktree marker. A marker that cannot be verified is an error rather
/// than a silent fallback: callers must not stamp work with a stale identity.
pub fn resolve(
    explicit: Option<&str>,
    fleet_env: Option<&str>,
    opencode_env: Option<&str>,
    marker: Option<&str>,
    marker_is_live: impl FnOnce(&str) -> bool,
) -> Result<Resolution, String> {
    if let Some(id) = nonempty(explicit) {
        return Ok(Resolution::Session(id.to_owned()));
    }
    if let Some(id) = nonempty(fleet_env).or_else(|| nonempty(opencode_env)) {
        return Ok(Resolution::Session(id.to_owned()));
    }
    let Some(id) = nonempty(marker) else {
        return Ok(Resolution::None);
    };
    if marker_is_live(id) {
        Ok(Resolution::Session(id.to_owned()))
    } else {
        Err(format!(
            "stale {PWD_FILE} refers to ended or unverifiable session {id}; remove it or start a new Fleet session"
        ))
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn worktree_root() -> Result<Option<PathBuf>, String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("cannot locate worktree root: {e}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if root.is_empty() {
        return Err("git returned an empty worktree root".into());
    }
    Ok(Some(PathBuf::from(root)))
}

fn marker_path(root: &Path) -> PathBuf {
    root.join(PWD_FILE)
}

fn resolve_marker(
    path: &Path,
    explicit: Option<&str>,
    fleet_env: Option<&str>,
    opencode_env: Option<&str>,
    marker_is_live: impl FnOnce(&str) -> bool,
) -> Result<Resolution, String> {
    let marker = match std::fs::read_to_string(path) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let resolved = resolve(
        explicit,
        fleet_env,
        opencode_env,
        marker.as_deref(),
        marker_is_live,
    )?;
    let supplied_identity = nonempty(explicit).is_some()
        || nonempty(fleet_env).is_some()
        || nonempty(opencode_env).is_some();
    if supplied_identity && let Resolution::Session(id) = &resolved {
        std::fs::write(path, format!("{id}\n"))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(resolved)
}

/// Best-effort liveness check against the local OpenCode server. An
/// unavailable server is not proof that the recorded session is live.
fn session_is_live(session_id: &str) -> bool {
    let base = std::env::var("OPENCODE_BASE_URL")
        .ok()
        .or_else(|| {
            std::env::var("OPENCODE_PORT")
                .ok()
                .map(|port| format!("http://127.0.0.1:{port}"))
        })
        .unwrap_or_else(|| "http://127.0.0.1:4096".to_owned());
    let url = format!("{}/api/session/{}", base.trim_end_matches('/'), session_id);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(2)))
        .build()
        .into();
    agent.get(&url).call().is_ok()
}

/// Set `FLEET_SESSION_ID` for Fleet's child processes and persist explicitly
/// supplied identities. With no supplied identity, a marker is used only when
/// the OpenCode session endpoint confirms it exists.
pub fn initialize(explicit: Option<&str>) -> Result<(), String> {
    let Some(root) = worktree_root()? else {
        return Ok(());
    };
    let path = marker_path(&root);
    let fleet_env = std::env::var("FLEET_SESSION_ID").ok();
    let opencode_env = std::env::var("OPENCODE_SESSION_ID").ok();
    let resolved = resolve_marker(
        &path,
        explicit,
        fleet_env.as_deref(),
        opencode_env.as_deref(),
        session_is_live,
    )?;
    if let Resolution::Session(id) = resolved {
        // Explicit values and recovered marker values apply to this process;
        // environment-sourced identities are also persisted by resolve_marker.
        unsafe { std::env::set_var("FLEET_SESSION_ID", &id) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_argument_wins_over_both_environments_and_pwd_file() {
        let got = resolve(
            Some("ses_explicit"),
            Some("ses_fleet"),
            Some("ses_open"),
            Some("ses_file"),
            |_| false,
        );
        assert_eq!(got, Ok(Resolution::Session("ses_explicit".into())));
    }

    #[test]
    fn fleet_env_precedes_opencode_env_and_pwd_file() {
        let got = resolve(
            None,
            Some("ses_fleet"),
            Some("ses_open"),
            Some("ses_file"),
            |_| false,
        );
        assert_eq!(got, Ok(Resolution::Session("ses_fleet".into())));
    }

    #[test]
    fn opencode_env_precedes_pwd_file() {
        let got = resolve(None, None, Some("ses_open"), Some("ses_file"), |_| false);
        assert_eq!(got, Ok(Resolution::Session("ses_open".into())));
    }

    #[test]
    fn marker_is_used_only_when_live_and_stale_marker_is_explicit() {
        assert_eq!(
            resolve(None, None, None, Some("ses_file"), |id| id == "ses_file"),
            Ok(Resolution::Session("ses_file".into()))
        );
        let error = resolve(None, None, None, Some("ses_ended"), |_| false).unwrap_err();
        assert!(error.contains("stale .fleet-session-id"));
        assert!(error.contains("ses_ended"));
    }

    #[test]
    fn whitespace_values_are_absent() {
        assert_eq!(
            resolve(None, Some("  "), None, None, |_| false),
            Ok(Resolution::None)
        );
    }

    #[test]
    fn supplied_identity_is_persisted_for_a_later_bare_invocation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from("/home/user/.local/share/opencode/tmp/opencode").join(format!(
            "fleet-session-state-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = marker_path(&root);

        assert_eq!(
            resolve_marker(&path, None, None, Some("ses_active"), |_| false),
            Ok(Resolution::Session("ses_active".into()))
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ses_active\n");
        assert_eq!(
            resolve_marker(&path, Some("ses_explicit"), None, None, |_| false),
            Ok(Resolution::Session("ses_explicit".into()))
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ses_explicit\n");
        assert_eq!(
            resolve_marker(&path, None, None, None, |id| id == "ses_explicit"),
            Ok(Resolution::Session("ses_explicit".into()))
        );

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
