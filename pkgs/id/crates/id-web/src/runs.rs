//! `POST /v1/runs`: run one ID subcommand over REST with argv parity.
//!
//! Slice 1 of the ID binary REST session and presentation design
//! (#10392 §6.1, build issue #14224; record
//! `doc/2026-11-24T08-40-00Z_design_id-rest-session-presentation-10392/`).
//!
//! A run is one request document (design §4.1):
//!
//! ```text
//! POST /v1/runs
//! {
//!   "argv": ["..."],                       // full argv after the binary name
//!   "env": { "NAME": "value" },            // allowlisted names only
//!   "stdin": { "mode": "none" }            // or "body" with "data" / "data_base64"
//! }
//! ```
//!
//! The run executes the serving binary itself with exactly that argv, so the
//! CLI is its own oracle: a REST run and a shell invocation with the same
//! argv, environment and input produce the same stdout, stderr and exit code
//! (design §7). The answer is one JSON document with the exit status and
//! both streams.
//!
//! Scope of this slice:
//!
//! - `stdin.mode = none` closes stdin at start; `stdin.mode = body` feeds the
//!   finite input carried in the request (`data` as UTF-8 text or
//!   `data_base64` as bytes) and then closes it. The mode is explicit, never
//!   inferred from whether input happens to be present.
//! - `stdin.mode = session` (interactive input, slice 3, #14227) runs in a
//!   session ([`crate::sessions`]): input arrives as numbered frames on the
//!   session's input endpoint, and the answer is the session's event
//!   stream, so it needs `Accept: text/event-stream` (406 otherwise). A
//!   streamed run with `body` input, or one naming a `session`, also runs
//!   in a session the client can attach to and replay.
//! - App runs (slice 4, #14226) go through the app loader ([`load_app`],
//!   §4.5): the request names an `app` the server has a [`RunManifest`]
//!   for, announces its `client` (`kind`, `features`), and lists the
//!   `capabilities` it grants. A run missing any required capability is
//!   refused with each one named and never starts; optional capabilities
//!   are granted only when listed. The grant reaches the run as
//!   environment ([`APP_ENV`], [`CAPABILITIES_ENV`], [`CLIENT_KIND_ENV`]).
//!   `GET /v1/apps/{app}/manifest` returns an app's manifest.
//! - Streaming output (slice 2, #14225, §4.2): a request whose `Accept`
//!   header names `text/event-stream` is answered as server-sent events as
//!   the run produces them. Each event has `id: <seq>` (from 1, increasing
//!   by one), `event: app | out | err | exit` and a JSON `data:` object
//!   ([`RunEventData::to_json`]); the `exit` event is last. A
//!   close-immediately run (`stdin.mode = none`, no session) is this one
//!   event stream in one HTTP response, and its output, exit status and
//!   truncation equal the plain JSON answer for the same request.
//!
//! Binding (design §4.7): a run executes arbitrary subcommands, so this
//! slice answers only requests whose peer address is loopback and that no
//! reverse proxy forwarded (a same-host proxy connects from loopback). A
//! request from any other address, one whose peer address is unknown, or one
//! carrying a forwarding header is refused with 403 before the body is read. Credentialed non-local runs come with
//! the session credential.
//!
//! Each run is bounded: it is killed when it exceeds the run timeout, and
//! each output stream keeps at most the stream cap (the rest is read and
//! dropped so the child never blocks on a full pipe), with the cut marked
//! `truncated`.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{ConnectInfo, FromRequest, Path, Request, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::sessions::SessionStore;

/// Path of the run endpoint.
pub const RUNS_PATH: &str = "/v1/runs";

/// Environment names a run request may set. Everything else is refused, so
/// a request can change how output looks and how much is logged, but never
/// where the binary reads or writes its data.
pub const ENV_ALLOWLIST: &[&str] = &[
    "COLUMNS",
    "DEBUG",
    "LANG",
    "LC_ALL",
    "LINES",
    "LOG_LEVEL",
    "NO_COLOR",
    "RUST_LOG",
    "TERM",
    "TZ",
];

/// Default wall-clock bound on one run.
pub const DEFAULT_RUN_TIMEOUT: Duration = Duration::from_secs(300);

/// Default number of bytes kept from each output stream.
pub const DEFAULT_STREAM_CAP: usize = 8 * 1024 * 1024;

/// How runs are executed.
#[derive(Clone, Debug)]
pub struct RunsConfig {
    /// The executable each run invokes: the serving `id` binary itself.
    pub exe: PathBuf,
    /// Wall-clock bound on one run; the child is killed past it.
    pub timeout: Duration,
    /// Bytes kept from each of stdout and stderr.
    pub stream_cap: usize,
    /// The app manifests the loader checks app runs against (§4.5).
    pub apps: RunApps,
    /// The sessions runs attach to (§4.3).
    pub sessions: Arc<SessionStore>,
}

impl RunsConfig {
    /// A config that runs `exe` with the default bounds.
    #[must_use]
    pub fn new(exe: PathBuf) -> Self {
        Self {
            exe,
            timeout: DEFAULT_RUN_TIMEOUT,
            stream_cap: DEFAULT_STREAM_CAP,
            apps: RunApps::new(BTreeMap::new()),
            sessions: Arc::new(SessionStore::default()),
        }
    }

    /// This config with `apps` as the loader's manifests.
    #[must_use]
    pub fn with_apps(mut self, apps: RunApps) -> Self {
        self.apps = apps;
        self
    }

    /// A config that runs the current executable, when it can be found.
    #[must_use]
    pub fn current_exe() -> Option<Self> {
        std::env::current_exe().ok().map(Self::new)
    }
}

/// The run endpoint as a router, ready to merge into any router state.
#[must_use]
pub fn runs_routes<S>(config: RunsConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(RUNS_PATH, post(run_handler))
        .route(APP_MANIFEST_PATH, get(manifest_handler))
        .merge(crate::sessions::session_routes())
        .merge(crate::presentation::presentation_routes())
        .with_state(Arc::new(config))
}

/// Path of an app's manifest (§4.5): `GET /v1/apps/{app}/manifest`.
pub const APP_MANIFEST_PATH: &str = "/v1/apps/{app}/manifest";

/// Environment names a granted app run receives (§4.5). They are not in
/// [`ENV_ALLOWLIST`], so a request cannot set them itself.
pub const APP_ENV: &str = "ID_RUN_APP";
/// The capabilities granted to an app run, comma-separated in manifest
/// order (required first, then the optional ones the client requested).
pub const CAPABILITIES_ENV: &str = "ID_RUN_CAPABILITIES";
/// The announced client kind of an app run.
pub const CLIENT_KIND_ENV: &str = "ID_RUN_CLIENT_KIND";

/// The capabilities an app declares (§4.5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestCapabilities {
    /// Capabilities the client must grant before the app starts.
    #[serde(default)]
    pub required: Vec<String>,
    /// Capabilities the app uses when the client grants them.
    #[serde(default)]
    pub optional: Vec<String>,
}

/// An app's run manifest (§4.5): what it needs before it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunManifest {
    /// The app id.
    pub app: String,
    /// The app version.
    pub version: String,
    /// Client-side assets the client gathers before it starts a session.
    #[serde(default)]
    pub client_side: Vec<String>,
    /// Required and optional capabilities.
    #[serde(default)]
    pub capabilities: ManifestCapabilities,
    /// How the app's pages reach the client (§4.6).
    #[serde(default)]
    pub pages: crate::presentation::Pages,
    /// The app folder its client-side assets are served from. Never
    /// serialized: clients see asset URLs, not server paths.
    #[serde(skip)]
    pub root: Option<PathBuf>,
}

/// The run manifests a server knows, keyed by app id.
pub type RunApps = Arc<BTreeMap<String, RunManifest>>;

/// The kind of client announcing itself (§4.1, §4.5 rule 1).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    /// A web frontend.
    Web,
    /// A WASI-style non-web client.
    Wasi,
    /// A command-line client.
    Cli,
    /// Any other client.
    Other,
}

impl ClientKind {
    /// The kind as it is spelled on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Wasi => "wasi",
            Self::Cli => "cli",
            Self::Other => "other",
        }
    }
}

/// A client's announcement of what it is and what it supports.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClientAnnouncement {
    /// What kind of client this is.
    pub kind: ClientKind,
    /// Features the client supports.
    #[serde(default)]
    pub features: Vec<String>,
}

/// The loader's decision for an app run that may start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppGrant {
    /// The app id.
    pub app: String,
    /// The announced client kind.
    pub client_kind: ClientKind,
    /// Granted capabilities: every required one, then each optional one the
    /// request granted, in manifest order.
    pub granted: Vec<String>,
}

/// Why the loader refused a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The HTTP status the refusal answers with.
    pub status: StatusCode,
    /// The reason, naming what is missing.
    pub message: String,
}

/// The app loader (§4.5): decide whether an app run may start.
///
/// A plain run (no `app`) needs no grant. An app run must name a known app
/// and announce its client; the loader refuses it, naming every missing
/// capability, when the request does not grant all of the app's required
/// capabilities, so the app never starts without them. Optional
/// capabilities are granted only when the request lists them; capabilities
/// the manifest does not declare are never granted.
///
/// # Errors
///
/// Returns the [`Refusal`] for an unknown app, a missing client
/// announcement, or a missing required capability.
pub fn load_app(apps: &RunApps, request: &RunRequest) -> Result<Option<AppGrant>, Refusal> {
    let Some(app) = &request.app else {
        return Ok(None);
    };
    let Some(manifest) = apps.get(app) else {
        return Err(Refusal {
            status: StatusCode::NOT_FOUND,
            message: format!("unknown app {app:?}"),
        });
    };
    let Some(client) = &request.client else {
        return Err(Refusal {
            status: StatusCode::BAD_REQUEST,
            message: format!("app {app:?} runs need a client announcement (client.kind)"),
        });
    };
    let offered = |name: &str| request.capabilities.iter().any(|have| have == name);
    let missing: Vec<&str> = manifest
        .capabilities
        .required
        .iter()
        .map(String::as_str)
        .filter(|name| !offered(name))
        .collect();
    if !missing.is_empty() {
        return Err(Refusal {
            status: StatusCode::FORBIDDEN,
            message: format!(
                "app {app:?} requires capabilities the client does not grant: {}",
                missing.join(", ")
            ),
        });
    }
    let mut granted: Vec<String> = Vec::new();
    for name in manifest.capabilities.required.iter().chain(
        manifest
            .capabilities
            .optional
            .iter()
            .filter(|name| offered(name.as_str())),
    ) {
        if !granted.contains(name) {
            granted.push(name.clone());
        }
    }
    Ok(Some(AppGrant {
        app: app.clone(),
        client_kind: client.kind,
        granted,
    }))
}

/// How the run's stdin is fed (design §4.1).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StdinMode {
    /// No input: stdin is closed at start.
    #[default]
    None,
    /// Finite piped input carried in the request, then EOF.
    Body,
    /// Open-ended interactive input through a session (not in this slice).
    Session,
}

/// The `stdin` member of a run request.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdinSpec {
    /// How input is fed.
    #[serde(default)]
    pub mode: StdinMode,
    /// `body` input as UTF-8 text.
    #[serde(default)]
    pub data: Option<String>,
    /// `body` input as base64-encoded bytes.
    #[serde(default)]
    pub data_base64: Option<String>,
}

/// A run request document (design §4.1).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// App id; omitted for a plain subcommand. App runs go through the
    /// loader ([`load_app`]).
    #[serde(default)]
    pub app: Option<String>,
    /// Full argv after the binary name.
    pub argv: Vec<String>,
    /// Allowlisted environment overrides.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// How stdin is fed.
    #[serde(default)]
    pub stdin: StdinSpec,
    /// Client announcement; app runs must carry one (§4.5 rule 1).
    #[serde(default)]
    pub client: Option<ClientAnnouncement>,
    /// Capabilities the client can grant, checked by the app loader.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// The session to attach the run to (§4.3); one is created when a
    /// streamed run has input (`stdin.mode` other than `none`) and names
    /// none.
    #[serde(default)]
    pub session: Option<String>,
}

/// One captured output stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamOutput {
    /// `utf8` when `data` is the text itself, `base64` when it encodes bytes.
    pub encoding: &'static str,
    /// The captured bytes, encoded as `encoding` says.
    pub data: String,
    /// Whether the stream ran past the stream cap and was cut.
    pub truncated: bool,
}

impl StreamOutput {
    fn from_bytes(bytes: Vec<u8>, truncated: bool) -> Self {
        match String::from_utf8(bytes) {
            Ok(data) => Self {
                encoding: "utf8",
                data,
                truncated,
            },
            Err(err) => Self {
                encoding: "base64",
                data: base64::engine::general_purpose::STANDARD.encode(err.into_bytes()),
                truncated,
            },
        }
    }
}

/// The answer to a finished run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RunResponse {
    /// The process exit code; absent when it was ended by a signal.
    pub exit_code: Option<i32>,
    /// The signal that ended the process, on Unix, if any.
    pub signal: Option<i32>,
    /// Whether the run was killed for exceeding the run timeout.
    pub timed_out: bool,
    /// Captured stdout.
    pub stdout: StreamOutput,
    /// Captured stderr.
    pub stderr: StreamOutput,
    /// The app this run started, for an app run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// The capabilities granted to an app run.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub granted_capabilities: Vec<String>,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

pub(crate) fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: message.into(),
        }),
    )
        .into_response()
}

/// Headers a reverse proxy adds when it forwards a request. A proxy on the
/// same host connects from loopback, so a request carrying any of these is
/// treated as non-local whatever its peer address.
const FORWARDING_HEADERS: &[&str] = &[
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-real-ip",
    "cf-connecting-ip",
];

/// Whether a request may run subcommands: a loopback peer, not forwarded
/// by a proxy (§4.7).
pub(crate) fn peer_allowed(peer: Option<SocketAddr>, headers: &axum::http::HeaderMap) -> bool {
    peer.is_some_and(|addr| addr.ip().is_loopback())
        && !FORWARDING_HEADERS
            .iter()
            .any(|name| headers.contains_key(*name))
}

/// Check a request against this slice's rules and return the stdin bytes
/// to feed (`None` for a closed stdin), or the reason it is refused.
///
/// # Errors
///
/// Returns the refusal message for a request this slice does not serve.
/// App checks are the loader's ([`load_app`]).
pub fn validate(request: &RunRequest) -> Result<Option<Vec<u8>>, String> {
    let refused: Vec<&str> = request
        .env
        .keys()
        .map(String::as_str)
        .filter(|name| !ENV_ALLOWLIST.contains(name))
        .collect();
    if !refused.is_empty() {
        return Err(format!(
            "env names not allowlisted: {}; allowed: {}",
            refused.join(", "),
            ENV_ALLOWLIST.join(", ")
        ));
    }
    let stdin = &request.stdin;
    match stdin.mode {
        StdinMode::Session => {
            if stdin.data.is_some() || stdin.data_base64.is_some() {
                Err(
                    "stdin.mode = session takes input as frames on the session's input endpoint, not data"
                        .to_owned(),
                )
            } else {
                Ok(None)
            }
        }
        StdinMode::None => {
            if stdin.data.is_some() || stdin.data_base64.is_some() {
                Err("stdin.mode = none takes no data; use mode = body to send input".to_owned())
            } else {
                Ok(None)
            }
        }
        StdinMode::Body => match (&stdin.data, &stdin.data_base64) {
            (Some(text), None) => Ok(Some(text.clone().into_bytes())),
            (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map(Some)
                .map_err(|err| format!("stdin.data_base64 is not valid base64: {err}")),
            (Some(_), Some(_)) => {
                Err("stdin.mode = body takes one of data or data_base64, not both".to_owned())
            }
            (None, None) => {
                Err("stdin.mode = body needs data or data_base64 (empty input is \"\")".to_owned())
            }
        },
    }
}

#[cfg(unix)]
fn exit_signal(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal()
}

#[cfg(not(unix))]
const fn exit_signal(_status: std::process::ExitStatus) -> Option<i32> {
    None
}

/// Execute one validated run.
///
/// # Errors
///
/// Fails when the executable cannot be started or its streams cannot be
/// read.
pub async fn execute(
    config: &RunsConfig,
    request: &RunRequest,
    stdin: Option<Vec<u8>>,
) -> std::io::Result<RunResponse> {
    execute_granted(config, request, stdin, None).await
}

/// Execute one validated run with the loader's grant, if it is an app run.
/// The grant reaches the run as [`APP_ENV`], [`CAPABILITIES_ENV`] and
/// [`CLIENT_KIND_ENV`].
///
/// # Errors
///
/// Fails when the executable cannot be started or its streams cannot be
/// read.
pub async fn execute_granted(
    config: &RunsConfig,
    request: &RunRequest,
    stdin: Option<Vec<u8>>,
    grant: Option<&AppGrant>,
) -> std::io::Result<RunResponse> {
    let started = start_run(config, request, stdin.into(), grant)?;
    let (events, mut received) = mpsc::channel::<RunEvent>(EVENT_BUFFER);
    let collect = async move {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit = None;
        while let Some(event) = received.recv().await {
            match event.data {
                RunEventData::Out(bytes) => stdout.extend(bytes),
                RunEventData::Err(bytes) => stderr.extend(bytes),
                RunEventData::Exit(end) => exit = Some(end),
                RunEventData::Session(_) | RunEventData::App(_) => {}
            }
        }
        (stdout, stderr, exit)
    };
    let (streamed, (stdout, stderr, exit)) = tokio::join!(started.stream(events), collect);
    streamed?;
    let exit = exit.ok_or_else(|| std::io::Error::other("run ended without an exit event"))?;
    Ok(RunResponse {
        exit_code: exit.exit_code,
        signal: exit.signal,
        timed_out: exit.timed_out,
        stdout: StreamOutput::from_bytes(stdout, exit.stdout_truncated),
        stderr: StreamOutput::from_bytes(stderr, exit.stderr_truncated),
        app: grant.map(|grant| grant.app.clone()),
        granted_capabilities: grant.map(|grant| grant.granted.clone()).unwrap_or_default(),
    })
}

/// Events a run may have in flight before its reader takes them.
pub(crate) const EVENT_BUFFER: usize = 16;

/// The `Accept` media type that asks for a run as server-sent events
/// (§4.2).
pub const EVENT_STREAM: &str = "text/event-stream";

/// One output event of a run (§4.2). Events are numbered by `seq` from 1,
/// in the order the run produced them; the `exit` event is always last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunEvent {
    /// The event's sequence number, from 1, increasing by one per event.
    pub seq: u64,
    /// What happened.
    pub data: RunEventData,
}

/// What a [`RunEvent`] carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEventData {
    /// The session a run belongs to; first in a session's log (§4.3).
    Session(String),
    /// The loader's grant for an app run; before any output.
    App(AppEvent),
    /// Bytes the run wrote to stdout.
    Out(Vec<u8>),
    /// Bytes the run wrote to stderr.
    Err(Vec<u8>),
    /// How the run ended; last.
    Exit(ExitEvent),
}

impl RunEventData {
    /// The SSE `event:` name: `app`, `out`, `err` or `exit`.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Session(_) | Self::App(_) => "app",
            Self::Out(_) => "out",
            Self::Err(_) => "err",
            Self::Exit(_) => "exit",
        }
    }

    /// The SSE `data:` JSON object. Output chunks are
    /// `{"encoding": "utf8" | "base64", "data": ...}`.
    ///
    /// # Errors
    ///
    /// Fails only if serialization fails.
    pub fn to_json(&self) -> serde_json::Result<String> {
        match self {
            Self::Session(id) => serde_json::to_string(&serde_json::json!({ "session": id })),
            Self::App(app) => serde_json::to_string(app),
            Self::Out(bytes) | Self::Err(bytes) => {
                serde_json::to_string(&ChunkData::from_bytes(bytes))
            }
            Self::Exit(exit) => serde_json::to_string(exit),
        }
    }
}

/// The `app` event: the loader's grant for an app run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppEvent {
    /// The app id.
    pub app: String,
    /// The announced client kind.
    pub client_kind: ClientKind,
    /// The capabilities granted to the run.
    pub granted_capabilities: Vec<String>,
}

/// The `exit` event: how the run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ExitEvent {
    /// The process exit code; absent when it was ended by a signal.
    pub exit_code: Option<i32>,
    /// The signal that ended the process, on Unix, if any.
    pub signal: Option<i32>,
    /// Whether the run was killed for exceeding the run timeout.
    pub timed_out: bool,
    /// Whether stdout went past the stream cap and was cut.
    pub stdout_truncated: bool,
    /// Whether stderr went past the stream cap and was cut.
    pub stderr_truncated: bool,
}

#[derive(Serialize)]
struct ChunkData {
    encoding: &'static str,
    data: String,
}

impl ChunkData {
    fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self {
                encoding: "utf8",
                data: text.to_owned(),
            },
            Err(_) => Self {
                encoding: "base64",
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
        }
    }
}

/// A run that has started and whose output has not been read yet.
#[derive(Debug)]
pub struct StartedRun {
    child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    stdin_task: Option<tokio::task::JoinHandle<()>>,
    timeout: Duration,
    stream_cap: usize,
    app: Option<AppEvent>,
}

/// Where a run's stdin comes from.
#[derive(Debug, Default)]
pub enum StdinFeed {
    /// No input: stdin is closed at start (`stdin.mode = none`).
    #[default]
    Closed,
    /// Finite input, then EOF (`stdin.mode = body`).
    Bytes(Vec<u8>),
    /// Open-ended input: each received chunk is written as it arrives, and
    /// the run sees EOF when the sender is dropped (`stdin.mode = session`).
    Frames(mpsc::Receiver<Vec<u8>>),
}

impl From<Option<Vec<u8>>> for StdinFeed {
    fn from(bytes: Option<Vec<u8>>) -> Self {
        bytes.map_or(Self::Closed, Self::Bytes)
    }
}

/// Start one validated run, with the loader's grant if it is an app run.
/// The grant reaches the run as [`APP_ENV`], [`CAPABILITIES_ENV`] and
/// [`CLIENT_KIND_ENV`]. Must be called inside a Tokio runtime.
///
/// # Errors
///
/// Fails when the executable cannot be started.
pub fn start_run(
    config: &RunsConfig,
    request: &RunRequest,
    stdin: StdinFeed,
    grant: Option<&AppGrant>,
) -> std::io::Result<StartedRun> {
    let mut command = tokio::process::Command::new(&config.exe);
    command.args(&request.argv).envs(&request.env);
    if let Some(grant) = grant {
        command
            .env(APP_ENV, &grant.app)
            .env(CAPABILITIES_ENV, grant.granted.join(","))
            .env(CLIENT_KIND_ENV, grant.client_kind.as_str());
    }
    command
        .stdin(if matches!(stdin, StdinFeed::Closed) {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;

    // A child that exits without reading its input closes the pipe; that
    // is the child's choice, not a run failure, so write errors end the
    // feed quietly.
    let stdin_task = match (child.stdin.take(), stdin) {
        (Some(mut writer), StdinFeed::Bytes(bytes)) => Some(tokio::spawn(async move {
            let _ = writer.write_all(&bytes).await;
            let _ = writer.shutdown().await;
        })),
        (Some(mut writer), StdinFeed::Frames(mut frames)) => Some(tokio::spawn(async move {
            while let Some(bytes) = frames.recv().await {
                if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                    return;
                }
            }
            let _ = writer.shutdown().await;
        })),
        _ => None,
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("child stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("child stderr was not piped"))?;
    Ok(StartedRun {
        child,
        stdout,
        stderr,
        stdin_task,
        timeout: config.timeout,
        stream_cap: config.stream_cap,
        app: grant.map(|grant| AppEvent {
            app: grant.app.clone(),
            client_kind: grant.client_kind,
            granted_capabilities: grant.granted.clone(),
        }),
    })
}

impl StartedRun {
    /// Read the run to its end and send its events, in order, to `events`:
    /// the `app` event for an app run, then `out` and `err` chunks as they
    /// arrive (each stream cut at the stream cap), then one `exit` event.
    /// When `events` closes early the run is killed and this returns.
    ///
    /// # Errors
    ///
    /// Fails when the run's streams cannot be read or its exit cannot be
    /// awaited; no `exit` event is sent then.
    // The sender is owned so the channel closes when the run ends.
    #[allow(clippy::needless_pass_by_value)]
    pub async fn stream(self, events: mpsc::Sender<RunEvent>) -> std::io::Result<()> {
        let Self {
            mut child,
            stdout,
            stderr,
            stdin_task,
            timeout,
            stream_cap,
            app,
        } = self;
        let (chunks, mut chunk_rx) = mpsc::channel(EVENT_BUFFER);
        let stdout_task = tokio::spawn(pump(stdout, stream_cap, chunks.clone(), RunEventData::Out));
        let stderr_task = tokio::spawn(pump(stderr, stream_cap, chunks, RunEventData::Err));
        let wait_task = tokio::spawn(async move {
            match tokio::time::timeout(timeout, child.wait()).await {
                Ok(status) => Ok::<_, std::io::Error>((status?, false)),
                Err(_elapsed) => {
                    child.kill().await?;
                    Ok((child.wait().await?, true))
                }
            }
        });
        // Whatever way this returns, nothing of the run outlives it: the
        // wait task owns the child, which is killed when it is dropped.
        let _guard = AbortOnDrop(
            [
                Some(stdout_task.abort_handle()),
                Some(stderr_task.abort_handle()),
                Some(wait_task.abort_handle()),
                stdin_task
                    .as_ref()
                    .map(tokio::task::JoinHandle::abort_handle),
            ]
            .into_iter()
            .flatten()
            .collect(),
        );

        let mut seq = 0_u64;
        if let Some(app) = app
            && !emit(&events, &mut seq, RunEventData::App(app)).await
        {
            return Ok(());
        }
        while let Some(chunk) = chunk_rx.recv().await {
            if !emit(&events, &mut seq, chunk).await {
                return Ok(());
            }
        }
        let stdout_truncated = stdout_task.await.map_err(std::io::Error::other)??;
        let stderr_truncated = stderr_task.await.map_err(std::io::Error::other)??;
        let (status, timed_out) = wait_task.await.map_err(std::io::Error::other)??;
        let exit = ExitEvent {
            exit_code: status.code(),
            signal: exit_signal(status),
            timed_out,
            stdout_truncated,
            stderr_truncated,
        };
        emit(&events, &mut seq, RunEventData::Exit(exit)).await;
        Ok(())
    }
}

/// Aborts its tasks when dropped.
struct AbortOnDrop(Vec<tokio::task::AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// Number and send one event; `false` when the receiver has gone.
async fn emit(events: &mpsc::Sender<RunEvent>, seq: &mut u64, data: RunEventData) -> bool {
    *seq += 1;
    events.send(RunEvent { seq: *seq, data }).await.is_ok()
}

/// Read `reader` to its end, sending at most `cap` bytes of it as chunks,
/// and report whether anything past the cap was dropped.
// The sender is owned so the channel closes when the run ends.
#[allow(clippy::needless_pass_by_value)]
async fn pump<R>(
    mut reader: R,
    cap: usize,
    chunks: mpsc::Sender<RunEventData>,
    wrap: fn(Vec<u8>) -> RunEventData,
) -> std::io::Result<bool>
where
    R: AsyncRead + Unpin,
{
    let mut sent = 0_usize;
    let mut truncated = false;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(truncated);
        }
        let room = cap.saturating_sub(sent);
        if read > room {
            truncated = true;
        }
        let keep = read.min(room);
        if keep > 0 {
            sent += keep;
            if chunks.send(wrap(buffer[..keep].to_vec())).await.is_err() {
                return Ok(truncated);
            }
        }
    }
}

/// Whether the request's `Accept` header asks for [`EVENT_STREAM`].
fn accepts_event_stream(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get_all(axum::http::header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|item| item.split(';').next())
        .any(|media| media.trim().eq_ignore_ascii_case(EVENT_STREAM))
}

/// Answer with the run's events as SSE, streamed as the run produces them.
fn event_stream_response(started: StartedRun) -> Response {
    let (events, received) = mpsc::channel(EVENT_BUFFER);
    tokio::spawn(async move {
        if let Err(err) = started.stream(events).await {
            tracing::warn!(error = %err, "run stream failed");
        }
    });
    let stream = futures::stream::unfold(received, |mut received| async move {
        let event = received.recv().await?;
        Some((
            Ok::<_, std::convert::Infallible>(sse_event(&event)),
            received,
        ))
    });
    Sse::new(stream).into_response()
}

pub(crate) fn sse_event(event: &RunEvent) -> Event {
    let data = event.data.to_json().unwrap_or_else(|_| String::from("{}"));
    Event::default()
        .id(event.seq.to_string())
        .event(event.data.name())
        .data(data)
}

async fn run_handler(State(config): State<Arc<RunsConfig>>, request: Request) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    if !peer_allowed(peer, request.headers()) {
        return error_response(
            StatusCode::FORBIDDEN,
            "runs are served to loopback clients only (#10392 §4.7)",
        );
    }
    let wants_events = accepts_event_stream(request.headers());
    let run = match Json::<RunRequest>::from_request(request, &()).await {
        Ok(Json(run)) => run,
        Err(rejection) => return rejection.into_response(),
    };
    let stdin = match validate(&run) {
        Ok(stdin) => stdin,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };
    let grant = match load_app(&config.apps, &run) {
        Ok(grant) => grant,
        Err(refusal) => return error_response(refusal.status, refusal.message),
    };
    // A run that needs a session (interactive input or a named session)
    // answers as events; a streamed run with input gets a session so the
    // client can resume it (§4.3).
    let needs_session = run.session.is_some() || run.stdin.mode == StdinMode::Session;
    if needs_session && !wants_events {
        return error_response(
            StatusCode::NOT_ACCEPTABLE,
            "session runs answer as server-sent events; send Accept: text/event-stream",
        );
    }
    if wants_events && (needs_session || run.stdin.mode == StdinMode::Body) {
        return crate::sessions::start_session_run(&config, &run, stdin, grant.as_ref()).await;
    }
    if wants_events {
        return match start_run(&config, &run, stdin.into(), grant.as_ref()) {
            Ok(started) => event_stream_response(started),
            Err(err) => {
                tracing::warn!(error = %err, "run could not be started");
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("run could not be started: {err}"),
                )
            }
        };
    }
    match execute_granted(&config, &run, stdin, grant.as_ref()).await {
        Ok(response) => Json(response).into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "run could not be executed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("run could not be executed: {err}"),
            )
        }
    }
}

async fn manifest_handler(
    State(config): State<Arc<RunsConfig>>,
    Path(app): Path<String>,
    ConnectInfoPeer(peer): ConnectInfoPeer,
    headers: axum::http::HeaderMap,
) -> Response {
    if !peer_allowed(peer, &headers) {
        return error_response(
            StatusCode::FORBIDDEN,
            "app manifests are served to loopback clients only (#10392 §4.7)",
        );
    }
    match config.apps.get(&app) {
        Some(manifest) => match crate::presentation::manifest_document(manifest) {
            Ok(document) => Json(document).into_response(),
            Err(error) => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("app {app:?} assets are unreadable: {error}"),
            ),
        },
        None => error_response(StatusCode::NOT_FOUND, format!("unknown app {app:?}")),
    }
}

/// The request's peer address, when the listener recorded one.
pub(crate) struct ConnectInfoPeer(pub(crate) Option<SocketAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for ConnectInfoPeer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt as _;

    fn sh_config() -> RunsConfig {
        RunsConfig::new(PathBuf::from("sh"))
    }

    fn loopback() -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], 40_000))
    }

    async fn post_run(
        config: RunsConfig,
        peer: Option<SocketAddr>,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let app: Router = runs_routes(config);
        let mut request = HttpRequest::post(RUNS_PATH)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(ConnectInfo(peer));
        }
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn captures_stdout_stderr_and_exit_code() {
        let (status, body) = post_run(
            sh_config(),
            Some(loopback()),
            &serde_json::json!({
                "argv": ["-c", "printf out; printf err >&2; exit 3"],
                "stdin": { "mode": "none" }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["exit_code"], 3);
        assert_eq!(body["timed_out"], false);
        assert_eq!(body["stdout"]["data"], "out");
        assert_eq!(body["stdout"]["encoding"], "utf8");
        assert_eq!(body["stderr"]["data"], "err");
    }

    #[tokio::test]
    async fn mode_none_closes_stdin() {
        let (status, body) = post_run(
            sh_config(),
            Some(loopback()),
            &serde_json::json!({ "argv": ["-c", "cat; echo done"] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["stdout"]["data"], "done\n");
    }

    #[tokio::test]
    async fn mode_body_feeds_text_and_bytes() {
        let (_, body) = post_run(
            sh_config(),
            Some(loopback()),
            &serde_json::json!({
                "argv": ["-c", "cat"],
                "stdin": { "mode": "body", "data": "piped\ninput" }
            }),
        )
        .await;
        assert_eq!(body["stdout"]["data"], "piped\ninput");

        let (_, body) = post_run(
            sh_config(),
            Some(loopback()),
            &serde_json::json!({
                "argv": ["-c", "cat"],
                "stdin": { "mode": "body", "data_base64": "/wA=" }
            }),
        )
        .await;
        assert_eq!(body["stdout"]["encoding"], "base64");
        assert_eq!(body["stdout"]["data"], "/wA=");
    }

    #[tokio::test]
    async fn allowlisted_env_reaches_the_run() {
        let (_, body) = post_run(
            sh_config(),
            Some(loopback()),
            &serde_json::json!({
                "argv": ["-c", "printf %s \"$NO_COLOR\""],
                "env": { "NO_COLOR": "1" }
            }),
        )
        .await;
        assert_eq!(body["stdout"]["data"], "1");
    }

    #[tokio::test]
    async fn refuses_unlisted_env_and_session_mode() {
        for request in [
            serde_json::json!({ "argv": [], "env": { "HOME": "/tmp" } }),
            serde_json::json!({ "argv": [], "stdin": { "mode": "session", "data": "x" } }),
            serde_json::json!({ "argv": [], "stdin": { "mode": "none", "data": "x" } }),
            serde_json::json!({ "argv": [], "stdin": { "mode": "body" } }),
        ] {
            let (status, body) = post_run(sh_config(), Some(loopback()), &request).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert!(body["error"].is_string(), "{body}");
        }
    }

    #[tokio::test]
    async fn refuses_non_loopback_and_unknown_peers() {
        let request = serde_json::json!({ "argv": ["-c", "echo ran"] });
        let remote = SocketAddr::from(([192, 0, 2, 7], 40_000));
        let (status, _) = post_run(sh_config(), Some(remote), &request).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = post_run(sh_config(), None, &request).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn refuses_proxied_requests_from_loopback() {
        let app: Router = runs_routes(sh_config());
        let mut request = HttpRequest::post(RUNS_PATH)
            .header("content-type", "application/json")
            .header("x-forwarded-for", "198.51.100.4")
            .body(Body::from(r#"{"argv":["-c","echo ran"]}"#))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(loopback()));
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn caps_output_and_marks_truncation() {
        let mut config = sh_config();
        config.stream_cap = 4;
        let (_, body) = post_run(
            config,
            Some(loopback()),
            &serde_json::json!({ "argv": ["-c", "printf 123456789"] }),
        )
        .await;
        assert_eq!(body["stdout"]["data"], "1234");
        assert_eq!(body["stdout"]["truncated"], true);
    }

    #[tokio::test]
    async fn kills_runs_past_the_timeout() {
        let mut config = sh_config();
        config.timeout = Duration::from_millis(200);
        let (status, body) = post_run(
            config,
            Some(loopback()),
            &serde_json::json!({ "argv": ["-c", "sleep 30"] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["timed_out"], true);
        assert!(body["exit_code"].is_null(), "{body}");
    }

    fn notes_apps() -> RunApps {
        let manifest = RunManifest {
            app: "notes".to_owned(),
            version: "1.0.0".to_owned(),
            client_side: vec!["assets/app.js".to_owned()],
            capabilities: ManifestCapabilities {
                required: vec!["fs.read".to_owned(), "net".to_owned()],
                optional: vec!["device.audio".to_owned()],
            },
            pages: crate::presentation::Pages::default(),
            root: None,
        };
        Arc::new(BTreeMap::from([("notes".to_owned(), manifest)]))
    }

    #[tokio::test]
    async fn loader_refuses_missing_required_capabilities_and_never_starts() {
        let marker =
            std::env::temp_dir().join(format!("id-runs-loader-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_file(&marker);
        let script = format!("touch '{}'", marker.display());
        let (status, body) = post_run(
            sh_config().with_apps(notes_apps()),
            Some(loopback()),
            &serde_json::json!({
                "app": "notes",
                "argv": ["-c", script],
                "client": { "kind": "web", "features": ["sse"] },
                "capabilities": ["fs.read"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        let error = body["error"].as_str().unwrap();
        assert!(error.contains("net"), "{error}");
        assert!(!error.contains("fs.read"), "{error}");
        assert!(!marker.exists(), "the refused app started");
    }

    #[tokio::test]
    async fn loader_grants_required_and_requested_optional_capabilities() {
        let (status, body) = post_run(
            sh_config().with_apps(notes_apps()),
            Some(loopback()),
            &serde_json::json!({
                "app": "notes",
                "argv": ["-c", "printf '%s|%s|%s' \"$ID_RUN_APP\" \"$ID_RUN_CAPABILITIES\" \"$ID_RUN_CLIENT_KIND\""],
                "client": { "kind": "wasi" },
                "capabilities": ["device.audio", "net", "fs.read", "undeclared"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["stdout"]["data"],
            "notes|fs.read,net,device.audio|wasi"
        );
        assert_eq!(body["app"], "notes");
        assert_eq!(
            body["granted_capabilities"],
            serde_json::json!(["fs.read", "net", "device.audio"])
        );
    }

    #[test]
    fn loader_grants_optional_capabilities_only_when_requested() {
        let request: RunRequest = serde_json::from_value(serde_json::json!({
            "app": "notes",
            "argv": [],
            "client": { "kind": "cli" },
            "capabilities": ["net", "fs.read"]
        }))
        .unwrap();
        let grant = load_app(&notes_apps(), &request).unwrap().unwrap();
        assert_eq!(grant.granted, vec!["fs.read".to_owned(), "net".to_owned()]);
        assert_eq!(grant.client_kind, ClientKind::Cli);
    }

    #[tokio::test]
    async fn loader_refuses_unknown_apps_and_missing_announcements() {
        let (status, _) = post_run(
            sh_config().with_apps(notes_apps()),
            Some(loopback()),
            &serde_json::json!({ "app": "nope", "argv": [], "client": { "kind": "web" } }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = post_run(
            sh_config().with_apps(notes_apps()),
            Some(loopback()),
            &serde_json::json!({ "app": "notes", "argv": [], "capabilities": ["fs.read", "net"] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, _) = post_run(
            sh_config().with_apps(notes_apps()),
            Some(loopback()),
            &serde_json::json!({ "argv": [], "client": { "kind": "toaster" } }),
        )
        .await;
        assert!(status.is_client_error(), "unknown client kind accepted");
    }

    #[tokio::test]
    async fn plain_runs_carry_no_grant() {
        let (_, body) = post_run(
            sh_config().with_apps(notes_apps()),
            Some(loopback()),
            &serde_json::json!({ "argv": ["-c", "printf %s \"${ID_RUN_APP-unset}\""] }),
        )
        .await;
        assert_eq!(body["stdout"]["data"], "unset");
        assert!(body.get("app").is_none(), "{body}");
        assert!(body.get("granted_capabilities").is_none(), "{body}");
    }

    async fn get_manifest(app: &str, peer: Option<SocketAddr>) -> (StatusCode, serde_json::Value) {
        let router: Router = runs_routes(sh_config().with_apps(notes_apps()));
        let mut request = HttpRequest::get(format!("/v1/apps/{app}/manifest"))
            .body(Body::empty())
            .unwrap();
        if let Some(peer) = peer {
            request.extensions_mut().insert(ConnectInfo(peer));
        }
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn manifest_route_serves_known_apps_to_loopback() {
        let (status, body) = get_manifest("notes", Some(loopback())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["app"], "notes");
        assert_eq!(
            body["capabilities"]["required"],
            serde_json::json!(["fs.read", "net"])
        );
        assert_eq!(body["client_side"], serde_json::json!(["assets/app.js"]));
        let (status, _) = get_manifest("nope", Some(loopback())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let remote = SocketAddr::from(([192, 0, 2, 7], 40_000));
        let (status, _) = get_manifest("notes", Some(remote)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    async fn post_events(
        config: RunsConfig,
        body: &serde_json::Value,
    ) -> (StatusCode, String, String) {
        let app: Router = runs_routes(config);
        let mut request = HttpRequest::post(RUNS_PATH)
            .header("content-type", "application/json")
            .header("accept", "text/html, text/event-stream;q=0.9")
            .body(Body::from(body.to_string()))
            .unwrap();
        request.extensions_mut().insert(ConnectInfo(loopback()));
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            content_type,
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    /// Parse an SSE body into `(id, event, data)` triples.
    fn parse_events(text: &str) -> Vec<(u64, String, serde_json::Value)> {
        text.split("\n\n")
            .filter(|block| !block.trim().is_empty())
            .map(|block| {
                let mut id = None;
                let mut event = None;
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("id:") {
                        id = Some(value.trim().parse().unwrap());
                    } else if let Some(value) = line.strip_prefix("event:") {
                        event = Some(value.trim().to_owned());
                    } else if let Some(value) = line.strip_prefix("data:") {
                        data.push_str(value.trim_start());
                    }
                }
                (
                    id.unwrap(),
                    event.unwrap(),
                    serde_json::from_str(&data).unwrap(),
                )
            })
            .collect()
    }

    /// The bytes an `out`/`err` event or a plain stream object carries.
    fn decoded(value: &serde_json::Value) -> Vec<u8> {
        let data = value["data"].as_str().unwrap();
        match value["encoding"].as_str().unwrap() {
            "utf8" => data.as_bytes().to_vec(),
            "base64" => base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap(),
            other => panic!("unknown encoding {other}"),
        }
    }

    fn stream_bytes(events: &[(u64, String, serde_json::Value)], name: &str) -> Vec<u8> {
        events
            .iter()
            .filter(|(_, event, _)| event == name)
            .flat_map(|(_, _, data)| decoded(data))
            .collect()
    }

    #[tokio::test]
    async fn sse_events_are_numbered_and_end_with_exit() {
        let (status, content_type, text) = post_events(
            sh_config(),
            &serde_json::json!({
                "argv": ["-c", "printf out; printf err >&2; printf more; exit 3"],
                "stdin": { "mode": "none" }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(content_type.starts_with(EVENT_STREAM), "{content_type}");
        let events = parse_events(&text);
        let ids: Vec<u64> = events.iter().map(|(id, _, _)| *id).collect();
        let expected: Vec<u64> = (1..=u64::try_from(ids.len()).unwrap()).collect();
        assert_eq!(ids, expected, "{text}");
        let exits: Vec<_> = events
            .iter()
            .filter(|(_, event, _)| event == "exit")
            .collect();
        assert_eq!(exits.len(), 1, "{text}");
        let (_, last, exit) = events.last().unwrap();
        assert_eq!(last, "exit");
        assert_eq!(exit["exit_code"], 3);
        assert_eq!(exit["timed_out"], false);
        assert_eq!(stream_bytes(&events, "out"), b"outmore");
        assert_eq!(stream_bytes(&events, "err"), b"err");
    }

    #[tokio::test]
    async fn close_immediately_stream_equals_plain_run() {
        let mut config = sh_config();
        config.stream_cap = 6;
        let request = serde_json::json!({
            "argv": ["-c", "printf 'a\\377b'; printf e >&2; printf 'cdefgh'; exit 2"]
        });
        let (status, plain) = post_run(config.clone(), Some(loopback()), &request).await;
        assert_eq!(status, StatusCode::OK, "{plain}");
        let (_, _, text) = post_events(config, &request).await;
        let events = parse_events(&text);
        let (_, _, exit) = events.last().unwrap();
        assert_eq!(stream_bytes(&events, "out"), decoded(&plain["stdout"]));
        assert_eq!(stream_bytes(&events, "err"), decoded(&plain["stderr"]));
        assert_eq!(decoded(&plain["stdout"]), b"a\xffbcde");
        assert_eq!(exit["exit_code"], plain["exit_code"]);
        assert_eq!(exit["signal"], plain["signal"]);
        assert_eq!(exit["timed_out"], plain["timed_out"]);
        assert_eq!(exit["stdout_truncated"], plain["stdout"]["truncated"]);
        assert_eq!(exit["stdout_truncated"], true);
        assert_eq!(exit["stderr_truncated"], plain["stderr"]["truncated"]);
    }

    #[tokio::test]
    async fn sse_app_runs_open_with_the_grant() {
        let (status, _, text) = post_events(
            sh_config().with_apps(notes_apps()),
            &serde_json::json!({
                "app": "notes",
                "argv": ["-c", "printf ran"],
                "client": { "kind": "web" },
                "capabilities": ["fs.read", "net"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let events = parse_events(&text);
        let (id, event, app) = &events[0];
        assert_eq!((*id, event.as_str()), (1, "app"));
        assert_eq!(app["app"], "notes");
        assert_eq!(app["client_kind"], "web");
        assert_eq!(
            app["granted_capabilities"],
            serde_json::json!(["fs.read", "net"])
        );
        assert_eq!(stream_bytes(&events, "out"), b"ran");
    }

    #[tokio::test]
    async fn sse_refusals_answer_before_any_event() {
        let (status, content_type, text) = post_events(
            sh_config().with_apps(notes_apps()),
            &serde_json::json!({
                "app": "notes",
                "argv": ["-c", "printf ran"],
                "client": { "kind": "web" }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
        assert!(
            content_type.starts_with("application/json"),
            "{content_type}"
        );
    }

    #[test]
    fn accept_negotiation_finds_the_event_stream() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!accepts_event_stream(&headers));
        headers.insert("accept", "application/json".parse().unwrap());
        assert!(!accepts_event_stream(&headers));
        headers.insert(
            "accept",
            "text/html, Text/Event-Stream; q=0.5".parse().unwrap(),
        );
        assert!(accepts_event_stream(&headers));
    }
}
