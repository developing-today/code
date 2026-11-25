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
//! - `stdin.mode = session` (interactive input, §4.3) is refused with a
//!   message naming the slice that adds it.
//! - App runs (slice 4, #14226) go through the app loader ([`load_app`],
//!   §4.5): the request names an `app` the server has a [`RunManifest`]
//!   for, announces its `client` (`kind`, `features`), and lists the
//!   `capabilities` it grants. A run missing any required capability is
//!   refused with each one named and never starts; optional capabilities
//!   are granted only when listed. The grant reaches the run as
//!   environment ([`APP_ENV`], [`CAPABILITIES_ENV`], [`CLIENT_KIND_ENV`]).
//!   `GET /v1/apps/{app}/manifest` returns an app's manifest.
//! - Streaming output (SSE, §4.2) is slice 2; this slice answers once, when
//!   the run ends.
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
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

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

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
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
fn peer_allowed(peer: Option<SocketAddr>, headers: &axum::http::HeaderMap) -> bool {
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
        StdinMode::Session => Err(
            "stdin.mode = session needs sessions (#10392 §6.3), which are not served yet"
                .to_owned(),
        ),
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

/// Read a stream to its end, keeping at most `cap` bytes; report whether
/// anything past the cap was dropped.
async fn read_capped<R>(mut reader: R, cap: usize) -> std::io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            return Ok((kept, truncated));
        }
        let room = cap.saturating_sub(kept.len());
        if read > room {
            truncated = true;
        }
        kept.extend_from_slice(&chunk[..read.min(room)]);
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
    let mut command = tokio::process::Command::new(&config.exe);
    command.args(&request.argv).envs(&request.env);
    if let Some(grant) = grant {
        command
            .env(APP_ENV, &grant.app)
            .env(CAPABILITIES_ENV, grant.granted.join(","))
            .env(CLIENT_KIND_ENV, grant.client_kind.as_str());
    }
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn()?;

    let stdin_task = match (child.stdin.take(), stdin) {
        (Some(mut writer), Some(bytes)) => Some(tokio::spawn(async move {
            // A child that exits without reading its input closes the
            // pipe; that is the child's choice, not a run failure.
            let _ = writer.write_all(&bytes).await;
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
    let cap = config.stream_cap;
    let stdout_task = tokio::spawn(read_capped(stdout, cap));
    let stderr_task = tokio::spawn(read_capped(stderr, cap));

    let (status, timed_out) = match tokio::time::timeout(config.timeout, child.wait()).await {
        Ok(status) => (status?, false),
        Err(_elapsed) => {
            child.kill().await?;
            (child.wait().await?, true)
        }
    };

    if let Some(task) = stdin_task {
        task.abort();
    }
    let (stdout, stdout_truncated) = stdout_task.await.map_err(std::io::Error::other)??;
    let (stderr, stderr_truncated) = stderr_task.await.map_err(std::io::Error::other)??;

    Ok(RunResponse {
        exit_code: status.code(),
        signal: exit_signal(status),
        timed_out,
        stdout: StreamOutput::from_bytes(stdout, stdout_truncated),
        stderr: StreamOutput::from_bytes(stderr, stderr_truncated),
        app: grant.map(|grant| grant.app.clone()),
        granted_capabilities: grant.map(|grant| grant.granted.clone()).unwrap_or_default(),
    })
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
        Some(manifest) => Json(manifest).into_response(),
        None => error_response(StatusCode::NOT_FOUND, format!("unknown app {app:?}")),
    }
}

/// The request's peer address, when the listener recorded one.
struct ConnectInfoPeer(Option<SocketAddr>);

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
            serde_json::json!({ "argv": [], "stdin": { "mode": "session" } }),
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
}
