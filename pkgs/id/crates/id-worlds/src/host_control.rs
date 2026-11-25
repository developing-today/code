//! The host's local control socket (#10696, packet P4 of the hosted-worlds
//! v1 design `doc/2026-11-16_design_id6283_hosted_worlds_v1.md`).
//!
//! `id host run` binds `host.sock` inside the data root and answers one
//! JSON request per connection, one line each way, so `id host start`,
//! `stop`, `archive`, `restore`, `list` and `inspect` act on the running
//! host. There is no network listener: the socket is a filesystem object
//! readable and writable by its owner alone, so only the operator who
//! runs the host can drive it. Requests are answered one at a time, in
//! the order they arrive, so two operator acts never interleave.

use std::future::Future;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::host::{LifecycleState, Supervisor, WakeMode, inspect_world, settings_body};

/// Name of the control socket inside a data root.
pub const CONTROL_SOCKET_NAME: &str = "host.sock";

/// Longest request line the host reads.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// Longest response line a client reads.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// How long the host waits for a connected client to send its request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Path of the control socket for `root`.
#[must_use]
pub fn socket_path(root: &Path) -> PathBuf {
    root.join(CONTROL_SOCKET_NAME)
}

/// One operator act, sent as one JSON line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlRequest {
    /// The per-state census.
    List,
    /// One world's record, as `id host inspect` shows it.
    Inspect {
        /// The world.
        world: String,
    },
    /// Asleep to live, by replay.
    Start {
        /// The world.
        world: String,
        /// Who asked, recorded in the host file.
        actor: String,
    },
    /// Live to asleep, after draining the world.
    Stop {
        /// The world.
        world: String,
        /// Who asked, recorded in the host file.
        actor: String,
    },
    /// Asleep to archived.
    Archive {
        /// The world.
        world: String,
        /// Who asked, recorded in the host file.
        actor: String,
    },
    /// Archived to asleep.
    Restore {
        /// The world.
        world: String,
        /// Who asked, recorded in the host file.
        actor: String,
    },
    /// Pin a world always-live, or unpin it (#10694).
    SetPinned {
        /// The world.
        world: String,
        /// Pinned or not.
        pinned: bool,
    },
    /// Set how a world wakes from asleep (#10694).
    SetWake {
        /// The world.
        world: String,
        /// The wake mode.
        wake: WakeMode,
    },
}

/// The host's answer, sent as one JSON line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlResponse {
    /// The act was done.
    pub ok: bool,
    /// Why it was refused, when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the act produced: a census, an inspection, or the world's
    /// new state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl ControlResponse {
    fn done(body: serde_json::Value) -> Self {
        Self {
            ok: true,
            error: None,
            body: Some(body),
        }
    }

    fn refused(error: String) -> Self {
        Self {
            ok: false,
            error: Some(error),
            body: None,
        }
    }
}

fn state_body(world: &str, state: LifecycleState) -> serde_json::Value {
    serde_json::json!({ "world": world, "state": state })
}

/// Perform one request against the supervisor.
pub async fn handle(supervisor: &mut Supervisor, request: ControlRequest) -> ControlResponse {
    let result: Result<serde_json::Value> = match request {
        ControlRequest::List => serde_json::to_value(supervisor.census()).map_err(Into::into),
        ControlRequest::Inspect { world } => {
            inspect_world(supervisor.root(), supervisor.file(), &world)
                .and_then(|seen| serde_json::to_value(seen).map_err(Into::into))
        }
        ControlRequest::Start { world, actor } => {
            supervisor.start(&world, &actor).await.map(|report| {
                serde_json::json!({
                    "world": world,
                    "state": LifecycleState::Live,
                    "sequence": report.sequence,
                    "journal_entries": report.journal_entries,
                    "replayed_inputs": report.replayed_inputs,
                    "program_error": report.program_error,
                })
            })
        }
        ControlRequest::Stop { world, actor } => supervisor
            .stop(&world, &actor)
            .await
            .map(|()| state_body(&world, LifecycleState::Asleep)),
        ControlRequest::Archive { world, actor } => supervisor
            .archive(&world, &actor)
            .map(|()| state_body(&world, LifecycleState::Archived)),
        ControlRequest::Restore { world, actor } => supervisor
            .restore(&world, &actor)
            .map(|()| state_body(&world, LifecycleState::Asleep)),
        ControlRequest::SetPinned { world, pinned } => supervisor
            .set_pinned(&world, pinned)
            .and_then(|()| settings_body(supervisor.file(), &world)),
        ControlRequest::SetWake { world, wake } => supervisor
            .set_wake(&world, wake)
            .and_then(|()| settings_body(supervisor.file(), &world)),
    };
    match result {
        Ok(body) => ControlResponse::done(body),
        Err(error) => ControlResponse::refused(format!("{error:#}")),
    }
}

/// Bind the control socket for `root`, readable and writable by its
/// owner alone. A socket file left by a host that is no longer running
/// is replaced.
///
/// # Errors
///
/// Refuses when a host already answers on the socket (two hosts must
/// never replay the same worlds), when something other than a socket
/// sits at its path, and on any filesystem error.
pub fn bind(root: &Path) -> Result<UnixListener> {
    let path = socket_path(root);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                bail!("a host is already running on {}", root.display());
            }
            std::fs::remove_file(&path)
                .with_context(|| format!("removing stale socket {}", path.display()))?;
        }
        Ok(_) => bail!(
            "{} exists and is not a socket: move it aside before running a host here",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("checking {}", path.display()));
        }
    }
    let listener =
        UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restricting {}", path.display()))?;
    Ok(listener)
}

/// Answer requests on `listener` one at a time until `shutdown` resolves.
///
/// # Errors
///
/// Never for a bad request or a client that hangs up: those are logged
/// and the next connection is answered.
pub async fn serve(
    supervisor: &mut Supervisor,
    listener: &UnixListener,
    shutdown: impl Future<Output = ()>,
) -> Result<()> {
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => return Ok(()),
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    if let Err(error) = answer(supervisor, stream).await {
                        tracing::warn!("host control: {error:#}");
                    }
                }
                Err(error) => tracing::warn!("host control: accept failed: {error}"),
            },
        }
    }
}

async fn answer(supervisor: &mut Supervisor, stream: UnixStream) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    tokio::time::timeout(REQUEST_TIMEOUT, reader.read_line(&mut line))
        .await
        .context("client sent no request in time")??;
    let response = match serde_json::from_str::<ControlRequest>(line.trim()) {
        Ok(request) => handle(supervisor, request).await,
        Err(error) => ControlResponse::refused(format!("bad request: {error}")),
    };
    let mut bytes = serde_json::to_vec(&response)?;
    bytes.push(b'\n');
    write.write_all(&bytes).await?;
    write.shutdown().await?;
    Ok(())
}

/// Send one request to the host running on `root` and wait for its answer.
///
/// # Errors
///
/// Fails when no host is running on `root`, or the connection or the
/// answer breaks. A refused act is an `Ok` response with `ok: false`.
pub async fn request(root: &Path, request: &ControlRequest) -> Result<ControlResponse> {
    try_request(root, request).await?.with_context(|| {
        format!(
            "no host is running on {} (start one with `id host run {}`)",
            root.display(),
            root.display()
        )
    })
}

/// As [`request`], but `Ok(None)` when no host answers on `root`: no
/// socket, or a socket left by a host that is gone.
///
/// # Errors
///
/// Fails when the socket cannot be reached for another reason, or the
/// connection or the answer breaks.
pub async fn try_request(root: &Path, request: &ControlRequest) -> Result<Option<ControlResponse>> {
    let path = socket_path(root);
    let stream = match UnixStream::connect(&path).await {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("connecting to {}", path.display()));
        }
    };
    let (read, mut write) = stream.into_split();
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    write.write_all(&bytes).await?;
    write.shutdown().await?;
    let mut reader = BufReader::new(read.take(MAX_RESPONSE_BYTES));
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    serde_json::from_str(line.trim())
        .context("reading the host's answer")
        .map(Some)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::host::{HostFile, LifecycleState};
    use tempfile::TempDir;

    fn data_root_with(names: &[&str]) -> TempDir {
        let root = TempDir::new().unwrap();
        for name in names {
            let dir = root.path().join(name);
            std::fs::create_dir_all(dir.join("modules")).unwrap();
            std::fs::write(dir.join("journal.jsonl"), "").unwrap();
        }
        root
    }

    fn act(op: &str, world: &str) -> ControlRequest {
        let world = world.to_owned();
        let actor = "ops".to_owned();
        match op {
            "start" => ControlRequest::Start { world, actor },
            "stop" => ControlRequest::Stop { world, actor },
            "archive" => ControlRequest::Archive { world, actor },
            "restore" => ControlRequest::Restore { world, actor },
            other => panic!("no op {other}"),
        }
    }

    #[tokio::test]
    async fn operator_acts_reach_a_running_host_over_its_socket() {
        let root = data_root_with(&["alpha", "beta", "gamma"]);
        let listener = bind(root.path()).unwrap();
        let mode = std::fs::metadata(socket_path(root.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let mut supervisor = Supervisor::open(root.path()).await.unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let path = root.path().to_path_buf();

        let script = async move {
            // A second host on the same root is refused while this one runs.
            assert!(bind(&path).is_err());

            let started = request(&path, &act("start", "alpha")).await.unwrap();
            assert!(started.ok, "{started:?}");
            assert_eq!(started.body.unwrap()["state"], "live");

            assert!(request(&path, &act("archive", "beta")).await.unwrap().ok);
            let refused = request(&path, &act("start", "beta")).await.unwrap();
            assert!(!refused.ok);
            assert!(refused.error.unwrap().contains("restore"));

            let census = request(&path, &ControlRequest::List).await.unwrap();
            let body = census.body.unwrap();
            assert_eq!(body["live"]["worlds"], 1);
            assert_eq!(body["asleep"]["worlds"], 1);
            assert_eq!(body["archived"]["worlds"], 1);

            let seen = request(
                &path,
                &ControlRequest::Inspect {
                    world: "beta".to_owned(),
                },
            )
            .await
            .unwrap();
            assert_eq!(seen.body.unwrap()["state"], "archived");

            assert!(request(&path, &act("restore", "beta")).await.unwrap().ok);
            assert!(request(&path, &act("stop", "alpha")).await.unwrap().ok);
            assert!(!request(&path, &act("stop", "nobody")).await.unwrap().ok);

            // A malformed line is answered, and the host keeps serving.
            let raw = UnixStream::connect(socket_path(&path)).await.unwrap();
            let (read, mut write) = raw.into_split();
            write.write_all(b"not json\n").await.unwrap();
            write.shutdown().await.unwrap();
            let mut answer = String::new();
            BufReader::new(read).read_line(&mut answer).await.unwrap();
            let bad: ControlResponse = serde_json::from_str(answer.trim()).unwrap();
            assert!(!bad.ok);
            assert!(bad.error.unwrap().starts_with("bad request"));
            assert!(request(&path, &ControlRequest::List).await.unwrap().ok);

            let pinned = request(
                &path,
                &ControlRequest::SetPinned {
                    world: "gamma".to_owned(),
                    pinned: true,
                },
            )
            .await
            .unwrap();
            assert!(pinned.ok, "{pinned:?}");
            assert_eq!(pinned.body.unwrap()["pinned"], true);
            let woke = request(
                &path,
                &ControlRequest::SetWake {
                    world: "gamma".to_owned(),
                    wake: WakeMode::Operator,
                },
            )
            .await
            .unwrap();
            assert_eq!(woke.body.unwrap()["wake"], "operator");
            let unknown = ControlRequest::SetPinned {
                world: "nobody".to_owned(),
                pinned: true,
            };
            assert!(!request(&path, &unknown).await.unwrap().ok);

            stop.send(()).unwrap();
        };
        let served = serve(&mut supervisor, &listener, async {
            stopped.await.unwrap();
        });
        let (served, ()) = tokio::join!(served, script);
        served.unwrap();
        supervisor.shutdown().await.unwrap();
        drop(listener);

        let file = HostFile::load(root.path()).unwrap();
        for name in ["alpha", "beta", "gamma"] {
            assert_eq!(file.worlds[name].state, LifecycleState::Asleep, "{name}");
        }
        assert!(file.worlds["gamma"].pinned);
        assert_eq!(file.worlds["gamma"].wake, WakeMode::Operator);
        let actors: Vec<&str> = file.worlds["beta"]
            .history
            .iter()
            .map(|change| change.actor.as_str())
            .collect();
        assert_eq!(actors, ["ops", "ops"]);

        // The host is gone: its socket file is stale and a new host binds.
        assert!(request(root.path(), &ControlRequest::List).await.is_err());
        assert!(
            try_request(root.path(), &ControlRequest::List)
                .await
                .unwrap()
                .is_none()
        );
        drop(bind(root.path()).unwrap());
    }

    #[test]
    fn bind_refuses_to_replace_a_file_that_is_not_a_socket() {
        let root = data_root_with(&[]);
        std::fs::write(socket_path(root.path()), b"keep me").unwrap();
        assert!(bind(root.path()).is_err());
        assert_eq!(std::fs::read(socket_path(root.path())).unwrap(), b"keep me");
    }

    #[test]
    fn requests_round_trip_as_tagged_json() {
        let line = serde_json::to_string(&act("archive", "w")).unwrap();
        assert_eq!(line, r#"{"op":"archive","world":"w","actor":"ops"}"#);
        let list: ControlRequest = serde_json::from_str(r#"{"op":"list"}"#).unwrap();
        assert_eq!(list, ControlRequest::List);
        let wake: ControlRequest =
            serde_json::from_str(r#"{"op":"set_wake","world":"w","wake":"operator"}"#).unwrap();
        assert_eq!(
            wake,
            ControlRequest::SetWake {
                world: "w".to_owned(),
                wake: WakeMode::Operator
            }
        );
    }
}
