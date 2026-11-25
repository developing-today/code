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
//! - `stdin.mode = session` (interactive input, §4.3) and `app` runs (the app
//!   loader, §4.5) are refused with a message naming the slice that adds
//!   them. `client` and `capabilities` are accepted and not yet checked.
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
use axum::extract::{ConnectInfo, FromRequest, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
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
}

impl RunsConfig {
    /// A config that runs `exe` with the default bounds.
    #[must_use]
    pub const fn new(exe: PathBuf) -> Self {
        Self {
            exe,
            timeout: DEFAULT_RUN_TIMEOUT,
            stream_cap: DEFAULT_STREAM_CAP,
        }
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
        .with_state(Arc::new(config))
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
    /// App id; omitted for a plain subcommand (app runs are slice 4).
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
    /// Client announcement (checked by the app loader, slice 4).
    #[serde(default)]
    pub client: Option<serde_json::Value>,
    /// Capabilities the client can grant (checked by the app loader, slice 4).
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
pub fn validate(request: &RunRequest) -> Result<Option<Vec<u8>>, String> {
    if let Some(app) = &request.app {
        return Err(format!(
            "app runs are not served yet (app loader, #10392 §6.4); got app {app:?}"
        ));
    }
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
    let mut command = tokio::process::Command::new(&config.exe);
    command
        .args(&request.argv)
        .envs(&request.env)
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
    match execute(&config, &run, stdin).await {
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
    async fn refuses_unlisted_env_session_mode_and_apps() {
        for request in [
            serde_json::json!({ "argv": [], "env": { "HOME": "/tmp" } }),
            serde_json::json!({ "argv": [], "stdin": { "mode": "session" } }),
            serde_json::json!({ "argv": [], "app": "notes" }),
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
}
