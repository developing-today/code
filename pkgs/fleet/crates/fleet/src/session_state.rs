//! Session identity inherited by Fleet commands and recovered per worktree.
//!
//! `.fleet-session-id` is deliberately a plain, one-line file at the Git
//! worktree root. It is not shared through the common Git directory, so each
//! worktree retains its own active session association.

#[cfg(feature = "opencode-probe")]
use base64::Engine;
use std::path::{Path, PathBuf};

pub const PWD_FILE: &str = ".fleet-session-id";
const USAGE: &str = "Usage: fleet session bind [--session-id ID]\n\nResolve and persist this worktree's session identity.";

/// This command owns its whole argument list. Validate before Git, marker I/O,
/// environment mutation or HTTP. In particular, never scan another domain's tail.
pub fn run(args: &[String]) -> i32 {
    run_with(args, initialize)
}

fn run_with<T>(args: &[String], initialize: impl FnOnce(Option<&str>) -> Result<T, String>) -> i32 {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{USAGE}");
        return 0;
    }
    let explicit = match parse_bind(args) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("fleet session: {error}\n{USAGE}");
            return 2;
        }
    };
    match initialize(explicit) {
        Ok(_) => 0,
        Err(error) => {
            eprintln!("fleet session: {error}");
            2
        }
    }
}

fn parse_bind(args: &[String]) -> Result<Option<&str>, String> {
    let value = match args {
        [verb] if verb == "bind" => return Ok(None),
        [verb, flag, value] if verb == "bind" && flag == "--session-id" => value.as_str(),
        [verb, flag] if verb == "bind" && flag.starts_with("--session-id=") => {
            &flag["--session-id=".len()..]
        }
        _ => return Err("expected bind with at most one --session-id value".into()),
    };
    if !valid_session_id(value) {
        return Err("session ID must be ses_ followed by ASCII letters or digits".into());
    }
    Ok(Some(value))
}

fn valid_session_id(value: &str) -> bool {
    value.starts_with("ses_")
        && value.len() > 4
        && value[4..].bytes().all(|c| c.is_ascii_alphanumeric())
}

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
        return Ok(Resolution::Session(checked_identity(id)?));
    }
    if let Some(id) = nonempty(fleet_env).or_else(|| nonempty(opencode_env)) {
        return Ok(Resolution::Session(checked_identity(id)?));
    }
    let Some(id) = nonempty(marker) else {
        return Ok(Resolution::None);
    };
    let id = checked_identity(id)?;
    if marker_is_live(&id) {
        Ok(Resolution::Session(id))
    } else {
        Err(format!(
            "stale {PWD_FILE} refers to ended or unverifiable session {id}; remove it or start a new Fleet session"
        ))
    }
}

fn checked_identity(id: &str) -> Result<String, String> {
    if valid_session_id(id) {
        Ok(id.to_owned())
    } else {
        Err("session ID must be ses_ followed by ASCII letters or digits".into())
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn worktree_root() -> Result<Option<PathBuf>, String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    worktree_root_from_output(output)
}

fn worktree_root_from_output(
    output: std::io::Result<std::process::Output>,
) -> Result<Option<PathBuf>, String> {
    // Fleet also runs outside a checkout (and in minimal installed
    // variants). A missing Git executable means there is no discoverable
    // worktree, not that unrelated Fleet commands should fail.
    let Ok(output) = output else {
        return Ok(None);
    };
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
    let supplied_identity = nonempty(explicit).is_some()
        || nonempty(fleet_env).is_some()
        || nonempty(opencode_env).is_some();
    let marker = if supplied_identity {
        None
    } else {
        match std::fs::read_to_string(path) {
            Ok(value) => Some(value),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
        }
    };
    let resolved = resolve(
        explicit,
        fleet_env,
        opencode_env,
        marker.as_deref(),
        marker_is_live,
    )?;
    if supplied_identity && let Resolution::Session(id) = &resolved {
        std::fs::write(path, format!("{id}\n"))
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    Ok(resolved)
}

/// Best-effort liveness check against the local OpenCode server. An
/// unavailable server is not proof that the recorded session is live.
#[cfg(feature = "opencode-probe")]
fn session_is_live(session_id: &str) -> bool {
    let base = resolve_opencode_base_url(
        std::env::var("OPENCODE_URL").ok().as_deref(),
        std::env::var("OPENCODE_SERVER_URL").ok().as_deref(),
    );
    let Some(password) = opencode_password() else {
        return false;
    };
    let auth = opencode_basic_auth(&password);
    let url = format!("{base}/api/session/{session_id}");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(true)
        .timeout_global(Some(std::time::Duration::from_secs(2)))
        .build()
        .into();
    let Ok(mut response) = agent.get(&url).header("Authorization", &auth).call() else {
        return false;
    };
    let Ok(body) = response.body_mut().read_to_string() else {
        return false;
    };
    session_record_is_live(&body, session_id)
}

/// Without the `opencode-probe` feature (the connect variant) there is
/// no HTTP client to ask, so a recorded session counts as not live:
/// the same answer an unreachable OpenCode server gives.
#[cfg(not(feature = "opencode-probe"))]
fn session_is_live(_session_id: &str) -> bool {
    false
}

#[cfg(feature = "opencode-probe")]
fn resolve_opencode_base_url(env_url: Option<&str>, env_server_url: Option<&str>) -> String {
    [env_url, env_server_url]
        .into_iter()
        .flatten()
        .map(|value| value.trim().trim_end_matches('/'))
        .find(|value| !value.is_empty())
        .unwrap_or("http://127.0.0.1:13420")
        .to_owned()
}

#[cfg(feature = "opencode-probe")]
fn resolve_opencode_auth_file(env_file: Option<&str>, home: &str) -> String {
    env_file
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{home}/.local/state/opencode/service-prod.json"))
}

#[cfg(feature = "opencode-probe")]
fn opencode_basic_auth(password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("opencode:{password}"))
    )
}

#[cfg(feature = "opencode-probe")]
fn opencode_password() -> Option<String> {
    let home = std::env::var("HOME")
        .ok()
        .filter(|value| !value.is_empty())?;
    let path =
        resolve_opencode_auth_file(std::env::var("OPENCODE_AUTH_FILE").ok().as_deref(), &home);
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    state.get("password")?.as_str().map(str::to_owned)
}

#[cfg(feature = "opencode-probe")]
fn session_record_is_live(body: &str, session_id: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    let record = value.get("data").unwrap_or(&value);
    record.get("id").and_then(|id| id.as_str()) == Some(session_id)
        && record
            .get("time")
            .and_then(|time| time.get("archived"))
            .is_none_or(|v| v.is_null())
}

/// Set `FLEET_SESSION_ID` for Fleet's child processes and persist explicitly
/// supplied identities. With no supplied identity, a marker is used only when
/// the OpenCode session endpoint confirms it exists.
pub fn initialize(explicit: Option<&str>) -> Result<Option<String>, String> {
    let fleet_env = std::env::var("FLEET_SESSION_ID").ok();
    let opencode_env = std::env::var("OPENCODE_SESSION_ID").ok();
    let root = worktree_root()?;
    let resolved = initialize_at(
        root.as_deref(),
        explicit,
        fleet_env.as_deref(),
        opencode_env.as_deref(),
        session_is_live,
    )?;
    if let Resolution::Session(id) = resolved {
        // Explicit values and recovered marker values apply to this process;
        // environment-sourced identities are also persisted by resolve_marker.
        unsafe { std::env::set_var("FLEET_SESSION_ID", &id) };
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

fn initialize_at(
    root: Option<&Path>,
    explicit: Option<&str>,
    fleet_env: Option<&str>,
    opencode_env: Option<&str>,
    marker_is_live: impl FnOnce(&str) -> bool,
) -> Result<Resolution, String> {
    match root {
        Some(root) => resolve_marker(
            &marker_path(root),
            explicit,
            fleet_env,
            opencode_env,
            marker_is_live,
        ),
        None => resolve(explicit, fleet_env, opencode_env, None, marker_is_live),
    }
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
    fn git_spawn_failure_means_no_discoverable_worktree() {
        let result = worktree_root_from_output(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "git missing",
        )));
        assert_eq!(
            result,
            Ok(None),
            "missing Git must not block Fleet commands"
        );
    }

    #[cfg(feature = "opencode-probe")]
    #[test]
    fn opencode_url_resolution_matches_fleet_queue_contract() {
        assert_eq!(
            resolve_opencode_base_url(Some(" https://one/ "), Some("https://two/")),
            "https://one"
        );
        assert_eq!(
            resolve_opencode_base_url(None, Some("https://two/")),
            "https://two"
        );
        assert_eq!(
            resolve_opencode_base_url(None, None),
            "http://127.0.0.1:13420"
        );
        assert_eq!(
            resolve_opencode_auth_file(Some(" /tmp/opencode-auth.json "), "/home/test"),
            "/tmp/opencode-auth.json"
        );
        assert_eq!(
            resolve_opencode_auth_file(None, "/home/test"),
            "/home/test/.local/state/opencode/service-prod.json"
        );
        assert_eq!(opencode_basic_auth("secret"), "Basic b3BlbmNvZGU6c2VjcmV0");
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
    fn invalid_environment_and_marker_ids_fail_closed() {
        assert!(resolve(None, Some("not-a-session"), None, None, |_| true).is_err());
        assert!(resolve(None, None, None, Some("ses_bad-id"), |_| true).is_err());
    }

    proptest::proptest! {
        #[test]
        fn session_id_format_accepts_only_nonempty_ascii_alphanumeric_suffix(suffix in ".*") {
            let expected = !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_alphanumeric());
            proptest::prop_assert_eq!(valid_session_id(&format!("ses_{suffix}")), expected);
        }
    }

    #[test]
    fn supplied_identity_is_persisted_for_a_later_bare_invocation() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
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

    #[test]
    fn explicit_and_environment_values_do_not_read_an_invalid_marker() {
        let root = std::env::temp_dir().join(format!(
            "fleet-session-invalid-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = marker_path(&root);
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert_eq!(
            resolve_marker(&path, Some("ses_explicit"), None, None, |_| false),
            Ok(Resolution::Session("ses_explicit".into()))
        );
        assert_eq!(
            resolve_marker(&path, None, Some("ses_env"), None, |_| false),
            Ok(Resolution::Session("ses_env".into()))
        );
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn no_worktree_still_resolves_environment_identity() {
        assert_eq!(
            initialize_at(None, None, Some("ses_env"), None, |_| false),
            Ok(Resolution::Session("ses_env".into()))
        );
        assert_eq!(
            initialize_at(None, Some("ses_explicit"), None, None, |_| false),
            Ok(Resolution::Session("ses_explicit".into()))
        );
    }

    #[test]
    fn bind_parser_owns_and_validates_its_arguments() {
        assert_eq!(parse_bind(&["bind".into()]), Ok(None));
        assert_eq!(
            parse_bind(&["bind".into(), "--session-id=ses_123".into()]),
            Ok(Some("ses_123"))
        );
        assert!(parse_bind(&["bind".into(), "--session-id".into()]).is_err());
        assert!(parse_bind(&["bind".into(), "--session-id".into(), "--help".into()]).is_err());
        assert!(parse_bind(&["bind".into(), "--session-id=ses_x".into(), "extra".into()]).is_err());
    }

    #[test]
    fn invalid_bind_does_not_invoke_session_initialization() {
        let mut invoked = false;
        let code = run_with(
            &["bind".into(), "--session-id".into(), "--help".into()],
            |_| {
                invoked = true;
                Ok(())
            },
        );
        assert_eq!(code, 2);
        assert!(!invoked);
    }

    #[cfg(feature = "opencode-probe")]
    #[test]
    fn session_record_requires_matching_non_archived_json() {
        assert!(session_record_is_live(
            r#"{"id":"ses_a","time":{}}"#,
            "ses_a"
        ));
        assert!(session_record_is_live(
            r#"{"data":{"id":"ses_a"}}"#,
            "ses_a"
        ));
        assert!(!session_record_is_live("<html>app</html>", "ses_a"));
        assert!(!session_record_is_live(r#"{"id":"ses_b"}"#, "ses_a"));
        assert!(!session_record_is_live(
            r#"{"id":"ses_a","time":{"archived":123}}"#,
            "ses_a"
        ));
    }
}
