//! The host's local control socket (#10696, packet P4 of the hosted-worlds
//! v1 design `doc/2026-11-16_design_id6283_hosted_worlds_v1.md`).
//!
//! `id host run` binds `host.sock` inside the data root and answers one
//! JSON request per connection, one line each way, so `id host start`,
//! `stop`, `archive`, `restore`, `list` and `inspect` act on the running
//! host. `id serve --world-host` binds the same socket and answers the
//! same requests for the worlds it serves ([`crate::node_control`]).
//! There is no network listener: the socket is a filesystem object
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

use crate::host::{
    LifecycleState, Supervisor, WakeMode, inspect_world, settings_body, snapshot_settings_body,
};
use crate::host_snapshots::SnapshotPolicy;
use crate::world_export::ExportManifest;

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
    /// Set the host-wide snapshot policy (no world) or one world's own;
    /// no policy clears it (#13739).
    SetSnapshots {
        /// The world, or none for the host-wide default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        world: Option<String>,
        /// The policy, or none to clear it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        policy: Option<SnapshotPolicy>,
    },
    /// Show the snapshot policy and sets, changing nothing (#13739).
    Snapshots {
        /// The world, or none for the host-wide default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        world: Option<String>,
    },
    /// Export a live world to a new file, signed by the host key (#10695).
    Export {
        /// The world.
        world: String,
        /// Absolute path of the file to write.
        path: PathBuf,
    },
    /// Restore an export file as a new asleep world (#10695).
    Import {
        /// Absolute path of the export file.
        path: PathBuf,
        /// Lowercase hex of the key the export must be signed by.
        source: String,
        /// Who asked, recorded in the host file.
        actor: String,
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
    pub(crate) const fn done(body: serde_json::Value) -> Self {
        Self {
            ok: true,
            error: None,
            body: Some(body),
        }
    }

    pub(crate) const fn refused(error: String) -> Self {
        Self {
            ok: false,
            error: Some(error),
            body: None,
        }
    }
}

/// Parse a pinned export source: 64 hex digits.
///
/// # Errors
///
/// Fails for anything that is not 32 bytes of hex.
pub fn parse_source(source: &str) -> Result<[u8; 32]> {
    crate::world_session::decode_hex(source)
        .and_then(|bytes| bytes.try_into().ok())
        .with_context(|| format!("source {source:?} is not 64 hex digits"))
}

pub(crate) fn export_body(manifest: &ExportManifest, path: &Path) -> Result<serde_json::Value> {
    let mut body = serde_json::to_value(manifest)?;
    body["path"] = serde_json::json!(path);
    Ok(body)
}

pub(crate) fn state_body(world: &str, state: LifecycleState) -> serde_json::Value {
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
        ControlRequest::SetSnapshots { world, policy } => supervisor
            .set_snapshot_policy(world.as_deref(), policy)
            .and_then(|()| {
                snapshot_settings_body(supervisor.root(), supervisor.file(), world.as_deref())
            }),
        ControlRequest::Snapshots { world } => {
            snapshot_settings_body(supervisor.root(), supervisor.file(), world.as_deref())
        }
        ControlRequest::Export { world, path } => supervisor
            .export(&world, &path)
            .await
            .and_then(|manifest| export_body(&manifest, &path)),
        ControlRequest::Import {
            path,
            source,
            actor,
        } => match parse_source(&source) {
            Ok(source) => supervisor
                .import(&path, source, &actor)
                .await
                .and_then(|manifest| export_body(&manifest, &path)),
            Err(error) => Err(error),
        },
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
    let mut snapshots = tokio::time::interval(Duration::from_secs(
        crate::host_snapshots::SNAPSHOT_TICK_SECS,
    ));
    snapshots.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = &mut shutdown => return Ok(()),
            _ = snapshots.tick() => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |since| since.as_secs());
                log_snapshot_run(&supervisor.run_due_snapshots(now).await);
            }
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

/// Log what a scheduled snapshot pass did, one line per set and failure.
fn log_snapshot_run(run: &crate::host_snapshots::SnapshotRun) {
    for set in &run.taken {
        tracing::info!(
            world = %set.world,
            entries = set.entries,
            bytes = set.bytes,
            "snapshot taken: {}",
            set.path.display()
        );
    }
    for set in &run.removed {
        tracing::info!(world = %set.world, "snapshot removed: {}", set.path.display());
    }
    for (world, error) in &run.failed {
        tracing::warn!(world = %world, "snapshot failed: {error}");
    }
}

async fn answer(supervisor: &mut Supervisor, stream: UnixStream) -> Result<()> {
    let (request, write) = read_request(stream).await?;
    let response = match request {
        Ok(request) => handle(supervisor, request).await,
        Err(refusal) => refusal,
    };
    write_response(write, &response).await
}

/// Read one request line from a connected client: the request, or the
/// refusal a line that does not parse gets, plus the half to answer on.
pub(crate) async fn read_request(
    stream: UnixStream,
) -> Result<(
    std::result::Result<ControlRequest, ControlResponse>,
    tokio::net::unix::OwnedWriteHalf,
)> {
    let (read, write) = stream.into_split();
    let mut reader = BufReader::new(read.take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    tokio::time::timeout(REQUEST_TIMEOUT, reader.read_line(&mut line))
        .await
        .context("client sent no request in time")??;
    let request = serde_json::from_str::<ControlRequest>(line.trim())
        .map_err(|error| ControlResponse::refused(format!("bad request: {error}")));
    Ok((request, write))
}

/// Send one response line and close the connection.
pub(crate) async fn write_response(
    mut write: tokio::net::unix::OwnedWriteHalf,
    response: &ControlResponse,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(response)?;
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
            "no host is running on {} (start one with `id host run {}`, or serve it with `id serve --world-host`)",
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
        let key = ed25519_dalek::SigningKey::from_bytes(&[5; 32]);
        let source = crate::world_session::encode_hex(key.verifying_key().as_bytes());
        supervisor.set_signing_key(key);
        let exports = TempDir::new().unwrap();
        let export_path = exports.path().join("alpha.export");
        let script_export = export_path.clone();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let path = root.path().to_path_buf();

        let script = async move {
            // A second host on the same root is refused while this one runs.
            assert!(bind(&path).is_err());

            let export = ControlRequest::Export {
                world: "alpha".to_owned(),
                path: script_export.clone(),
            };
            let asleep = request(&path, &export).await.unwrap();
            assert!(asleep.error.unwrap().contains("start it"));

            let started = request(&path, &act("start", "alpha")).await.unwrap();
            assert!(started.ok, "{started:?}");
            assert_eq!(started.body.unwrap()["state"], "live");

            let exported = request(&path, &export).await.unwrap();
            assert!(exported.ok, "{exported:?}");
            assert_eq!(exported.body.unwrap()["world"], "alpha");
            // Restoring over a world this root records is refused.
            let again = ControlRequest::Import {
                path: script_export.clone(),
                source: source.clone(),
                actor: "ops".to_owned(),
            };
            let refused = request(&path, &again).await.unwrap();
            assert!(refused.error.unwrap().contains("already"), "refused");

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

        // The export restores into another data root with no host running.
        let elsewhere = data_root_with(&[]);
        let restored = crate::host::import_into_root(
            elsewhere.path(),
            &export_path,
            parse_source(&source_hex_of(5)).unwrap(),
            "ops",
        )
        .await
        .unwrap();
        assert_eq!(restored.world, "alpha");
        let there = HostFile::load(elsewhere.path()).unwrap();
        assert_eq!(there.worlds["alpha"].state, LifecycleState::Asleep);
        assert_eq!(there.worlds["alpha"].history[0].actor, "ops");
        let mut moved = Supervisor::open(elsewhere.path()).await.unwrap();
        moved.start("alpha", "ops").await.unwrap();
        moved.shutdown().await.unwrap();

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

    fn source_hex_of(seed: u8) -> String {
        let key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        crate::world_session::encode_hex(key.verifying_key().as_bytes())
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
        assert!(parse_source(&source_hex_of(1)).is_ok());
        assert!(parse_source("abc").is_err());
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
