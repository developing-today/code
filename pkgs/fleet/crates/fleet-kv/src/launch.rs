//! The trusted launch-identity record behind `fleet kv`.
//!
//! Every local row is keyed by the caller's complete identity tuple, so the
//! owner and context must come from somewhere the caller cannot simply
//! claim on the command line or in the environment. Phase 1 reads them from
//! a persisted launch record that the session launcher writes once, at
//! session start:
//!
//! ```text
//! <fleet state dir>/kv/launch/<session id>.identity
//! ```
//!
//! The record is trusted only when it is a regular file (not a symlink)
//! owned by this user, not writable by group or others, names the session
//! it is filed under, and carries well-formed values. Anything else fails
//! closed with `identity_unavailable`; there is no fallback to `FLEET_CTX_*`
//! or any other client-supplied override. The partition is the session.

use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::Identity;

const FORMAT_LINE: &str = "fleet-kv-launch-identity v1";
const MAX_VALUE_BYTES: usize = 128;

/// The identity tuple a `fleet kv` invocation runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchIdentity {
    pub owner_user_id: String,
    pub context_id: String,
    pub partition_id: String,
}

impl LaunchIdentity {
    pub fn identity(&self) -> Identity<'_> {
        Identity::new(&self.owner_user_id, &self.context_id, &self.partition_id)
    }
}

/// Why no trusted identity is available. Every variant refuses with the
/// one typed code `identity_unavailable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    NoSession,
    BadSession,
    Missing(PathBuf),
    Untrusted(String),
    Malformed(String),
    Io(String),
}

impl LaunchError {
    pub fn code(&self) -> &'static str {
        "identity_unavailable"
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSession => f.write_str(
                "no fleet session is bound here, so there is no launch identity to scope keys by",
            ),
            Self::BadSession => f.write_str("the bound session id is not a valid fleet session id"),
            Self::Missing(path) => write!(
                f,
                "no launch-identity record for this session at {}",
                path.display()
            ),
            Self::Untrusted(why) => write!(f, "launch-identity record refused: {why}"),
            Self::Malformed(why) => write!(f, "launch-identity record is malformed: {why}"),
            Self::Io(why) => write!(f, "launch-identity record unreadable: {why}"),
        }
    }
}

impl std::error::Error for LaunchError {}

fn valid_session_id(value: &str) -> bool {
    value.len() > 4
        && value.len() <= MAX_VALUE_BYTES
        && value.starts_with("ses_")
        && value[4..].bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn valid_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_VALUE_BYTES
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

/// Where the record for `session_id` lives under the fleet state dir.
pub fn record_path(state_dir: &Path, session_id: &str) -> PathBuf {
    state_dir
        .join("kv")
        .join("launch")
        .join(format!("{session_id}.identity"))
}

/// Where the host-local KV database lives under the fleet state dir.
pub fn database_path(state_dir: &Path) -> PathBuf {
    state_dir.join("kv").join("local.db")
}

/// Load and validate the launch identity for the bound session.
pub fn load(state_dir: &Path, session_id: Option<&str>) -> Result<LaunchIdentity, LaunchError> {
    let session_id = session_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or(LaunchError::NoSession)?;
    if !valid_session_id(session_id) {
        return Err(LaunchError::BadSession);
    }
    let path = record_path(state_dir, session_id);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(LaunchError::Missing(path));
        }
        Err(error) => return Err(LaunchError::Io(error.to_string())),
    };
    if !metadata.file_type().is_file() {
        return Err(LaunchError::Untrusted("not a regular file".into()));
    }
    check_ownership(&metadata)?;
    let text =
        std::fs::read_to_string(&path).map_err(|error| LaunchError::Io(error.to_string()))?;
    parse(&text, session_id)
}

#[cfg(unix)]
fn check_ownership(metadata: &std::fs::Metadata) -> Result<(), LaunchError> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    if metadata.uid() != euid {
        return Err(LaunchError::Untrusted("owned by another user".into()));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(LaunchError::Untrusted("writable by group or others".into()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_ownership(_metadata: &std::fs::Metadata) -> Result<(), LaunchError> {
    Err(LaunchError::Untrusted(
        "ownership cannot be verified on this platform".into(),
    ))
}

fn parse(text: &str, session_id: &str) -> Result<LaunchIdentity, LaunchError> {
    let mut lines = text.lines();
    if lines.next() != Some(FORMAT_LINE) {
        return Err(LaunchError::Malformed("unknown format line".into()));
    }
    let mut session = None;
    let mut owner = None;
    let mut context = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (field, value) = line
            .split_once('=')
            .ok_or_else(|| LaunchError::Malformed("a line has no `=`".into()))?;
        if !valid_value(value) {
            return Err(LaunchError::Malformed(format!("`{field}` has a bad value")));
        }
        let slot = match field {
            "session_id" => &mut session,
            "owner_user_id" => &mut owner,
            "context_id" => &mut context,
            _ => return Err(LaunchError::Malformed(format!("unknown field `{field}`"))),
        };
        if slot.replace(value.to_string()).is_some() {
            return Err(LaunchError::Malformed(format!("`{field}` given twice")));
        }
    }
    let missing = |name: &str| LaunchError::Malformed(format!("`{name}` is missing"));
    let session = session.ok_or_else(|| missing("session_id"))?;
    if session != session_id {
        return Err(LaunchError::Untrusted(
            "it names a different session".into(),
        ));
    }
    Ok(LaunchIdentity {
        owner_user_id: owner.ok_or_else(|| missing("owner_user_id"))?,
        context_id: context.ok_or_else(|| missing("context_id"))?,
        partition_id: session,
    })
}

/// Write the record for a session, owner-only, replacing any earlier one
/// atomically. For the session launcher; `fleet kv` itself never writes it.
pub fn write(
    state_dir: &Path,
    session_id: &str,
    owner_user_id: &str,
    context_id: &str,
) -> Result<PathBuf, LaunchError> {
    if !valid_session_id(session_id) {
        return Err(LaunchError::BadSession);
    }
    for (name, value) in [("owner_user_id", owner_user_id), ("context_id", context_id)] {
        if !valid_value(value) {
            return Err(LaunchError::Malformed(format!("`{name}` has a bad value")));
        }
    }
    let path = record_path(state_dir, session_id);
    let dir = path
        .parent()
        .ok_or_else(|| LaunchError::Io("record path has no parent".into()))?;
    std::fs::create_dir_all(dir).map_err(|error| LaunchError::Io(error.to_string()))?;
    let temp = dir.join(format!(".{session_id}.identity.{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp)?;
        write!(
            file,
            "{FORMAT_LINE}\nsession_id={session_id}\nowner_user_id={owner_user_id}\ncontext_id={context_id}\n"
        )?;
        file.sync_all()?;
        std::fs::rename(&temp, &path)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(LaunchError::Io(error.to_string()));
    }
    Ok(path)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("fleet-kv-launch-{}-{suffix}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn written_records_load_as_the_session_partition() {
        let dir = scratch();
        write(&dir, "ses_abc123", "user-1", "ctx-9").expect("write");
        let loaded = load(&dir, Some("ses_abc123")).expect("load");
        assert_eq!(
            loaded,
            LaunchIdentity {
                owner_user_id: "user-1".into(),
                context_id: "ctx-9".into(),
                partition_id: "ses_abc123".into(),
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_session_or_record_fails_closed() {
        let dir = scratch();
        assert_eq!(load(&dir, None), Err(LaunchError::NoSession));
        assert_eq!(load(&dir, Some("  ")), Err(LaunchError::NoSession));
        assert_eq!(
            load(&dir, Some("../../etc/passwd")),
            Err(LaunchError::BadSession)
        );
        assert!(matches!(
            load(&dir, Some("ses_none")),
            Err(LaunchError::Missing(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_record_filed_under_another_session_is_refused() {
        let dir = scratch();
        let written = write(&dir, "ses_one", "user", "ctx").expect("write");
        let other = record_path(&dir, "ses_two");
        std::fs::copy(&written, &other).expect("copy");
        assert!(matches!(
            load(&dir, Some("ses_two")),
            Err(LaunchError::Untrusted(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn group_writable_and_symlinked_records_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch();
        let path = write(&dir, "ses_perm", "user", "ctx").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o620)).expect("chmod");
        assert!(matches!(
            load(&dir, Some("ses_perm")),
            Err(LaunchError::Untrusted(_))
        ));
        let real = write(&dir, "ses_real", "user", "ctx").expect("write");
        std::os::unix::fs::symlink(&real, record_path(&dir, "ses_link")).expect("symlink");
        assert!(matches!(
            load(&dir, Some("ses_link")),
            Err(LaunchError::Untrusted(_))
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parsing_is_strict() {
        let good = format!("{FORMAT_LINE}\nsession_id=ses_x\nowner_user_id=u\ncontext_id=c\n");
        assert!(parse(&good, "ses_x").is_ok());
        for bad in [
            "session_id=ses_x\nowner_user_id=u\ncontext_id=c\n".to_string(),
            format!("{FORMAT_LINE}\nsession_id=ses_x\nowner_user_id=u\n"),
            format!(
                "{FORMAT_LINE}\nsession_id=ses_x\nowner_user_id=u\nowner_user_id=v\ncontext_id=c\n"
            ),
            format!("{FORMAT_LINE}\nsession_id=ses_x\nowner_user_id=has space\ncontext_id=c\n"),
            format!("{FORMAT_LINE}\nsession_id=ses_x\nowner_user_id=u\ncontext_id=c\nextra=1\n"),
            format!("{FORMAT_LINE}\nsession_id=ses_x\nowner_user_id=\ncontext_id=c\n"),
        ] {
            assert!(
                matches!(parse(&bad, "ses_x"), Err(LaunchError::Malformed(_))),
                "{bad:?}"
            );
        }
    }
}
