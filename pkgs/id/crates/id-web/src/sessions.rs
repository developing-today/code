//! Run sessions, input frames and replay (#10392 §4.3, §4.4; slice 3,
//! #14227).
//!
//! A session holds one run's ordered event log. `POST /v1/sessions` answers
//! `{ "id": ... }` at once, before any output; a run attaches to it with
//! `POST /v1/runs` naming `session`, and a streamed run with input
//! (`stdin.mode` `body` or `session`) that names none gets a new session.
//! The run's answer is the session's event stream, whose first event is
//! `event: app` with `{ "session": id }`.
//!
//! - **Input** (`stdin.mode = session`): numbered frames
//!   `POST /v1/sessions/{id}/input` with `{ seq, bytes, close }` (`bytes`
//!   base64, `seq` from 1). Frames are accepted in order; a gap and a
//!   conflicting repeat are refused with distinct kinds (`gap`,
//!   `conflict`); a repeat with identical content is accepted again without
//!   effect. A frame with `close: true` ends input, and the run sees EOF.
//! - **Attach and replay**: `GET /v1/sessions/{id}/events` answers SSE with
//!   the replay modes of §4.4 (`since`, `after`, `before` with `after`,
//!   `all=1`; `Last-Event-ID` also works as `since`). Without `before` it
//!   follows the run live until its `exit` event.
//! - **WebSocket**: `GET /v1/sessions/{id}/socket` upgrades to a socket that
//!   carries the same objects: each event as `{ id, event, data }`, and each
//!   input frame sent by the client answered with the same JSON the input
//!   endpoint answers.
//! - **Retention**: each log keeps at most a byte cap of events (default
//!   16 MiB) and a session is dropped after an idle TTL (default one hour,
//!   at most 24 hours). When the cap drops old events, a reader whose
//!   cursor is below the first retained `seq` gets a truncation marker
//!   (`event: app` with `{ "truncated": true, "first_retained_seq": n }`)
//!   before the events, never a silent gap.
//!
//! Replay is a pure function of the log ([`EventLog::replay`]): for any
//! split point `k`, the window `after=0, before=k+1` followed by `since=k`
//! is the whole log. Sessions are served to loopback clients only, like
//! runs (§4.7).

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::{mpsc, watch};

use crate::runs::{
    AppGrant, ConnectInfoPeer, EVENT_BUFFER, RunEvent, RunEventData, RunRequest, RunsConfig,
    StdinFeed, StdinMode, error_response, peer_allowed, sse_event, start_run,
};

/// Create a session: `POST /v1/sessions`.
pub const SESSIONS_PATH: &str = "/v1/sessions";
/// Send an input frame: `POST /v1/sessions/{id}/input`.
pub const SESSION_INPUT_PATH: &str = "/v1/sessions/{id}/input";
/// Attach or replay as SSE: `GET /v1/sessions/{id}/events`.
pub const SESSION_EVENTS_PATH: &str = "/v1/sessions/{id}/events";
/// Attach over a WebSocket: `GET /v1/sessions/{id}/socket`.
pub const SESSION_SOCKET_PATH: &str = "/v1/sessions/{id}/socket";

/// Default byte cap of one session's event log (§4.4).
pub const DEFAULT_SESSION_BYTE_CAP: usize = 16 * 1024 * 1024;
/// Default idle time after which a session is dropped (§4.4).
pub const DEFAULT_SESSION_IDLE_TTL: Duration = Duration::from_hours(1);
/// The longest idle TTL a store accepts (§4.4).
pub const MAX_SESSION_IDLE_TTL: Duration = Duration::from_hours(24);

/// Bytes counted for each event on top of its payload.
const EVENT_OVERHEAD: usize = 32;

/// Which events a reader asks for (§4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayQuery {
    /// Events with `seq > since` (`?since=` and `?after=`).
    Since(u64),
    /// Events with `after < seq < before` (`?before=&after=`).
    Window {
        /// Exclusive lower bound.
        after: u64,
        /// Exclusive upper bound.
        before: u64,
    },
    /// The whole retained log (`?all=1`).
    All,
}

/// The answer to a [`ReplayQuery`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    /// The first retained `seq`, when the query reaches below it: events
    /// it asked for were dropped by retention.
    pub truncated: Option<u64>,
    /// The retained events the query asked for, in order.
    pub events: Vec<RunEvent>,
}

/// One session's ordered, bounded event log (§4.4).
#[derive(Clone, Debug)]
pub struct EventLog {
    events: VecDeque<(RunEvent, usize)>,
    bytes: usize,
    cap: usize,
    last_seq: u64,
    finished: bool,
}

impl EventLog {
    /// An empty log that keeps at most `cap` bytes of events (always at
    /// least the newest one).
    #[must_use]
    pub const fn new(cap: usize) -> Self {
        Self {
            events: VecDeque::new(),
            bytes: 0,
            cap,
            last_seq: 0,
            finished: false,
        }
    }

    /// Append an event, numbering it after the last one, and drop the
    /// oldest events past the byte cap. Returns its `seq`.
    pub fn push(&mut self, data: RunEventData) -> u64 {
        if matches!(data, RunEventData::Exit(_)) {
            self.finished = true;
        }
        self.last_seq += 1;
        let size = event_size(&data);
        self.events.push_back((
            RunEvent {
                seq: self.last_seq,
                data,
            },
            size,
        ));
        self.bytes += size;
        while self.bytes > self.cap && self.events.len() > 1 {
            if let Some((_, dropped)) = self.events.pop_front() {
                self.bytes -= dropped;
            }
        }
        self.last_seq
    }

    /// The first retained `seq` (one past the last when the log is empty).
    #[must_use]
    pub fn floor(&self) -> u64 {
        self.events
            .front()
            .map_or(self.last_seq + 1, |(event, _)| event.seq)
    }

    /// The last `seq` given out, or 0.
    #[must_use]
    pub const fn last_seq(&self) -> u64 {
        self.last_seq
    }

    /// Whether the log holds the run's `exit` event.
    #[must_use]
    pub const fn finished(&self) -> bool {
        self.finished
    }

    /// Answer a replay query; a pure function of the log.
    #[must_use]
    pub fn replay(&self, query: ReplayQuery) -> Replay {
        let (after, before) = match query {
            ReplayQuery::Since(since) => (since, None),
            ReplayQuery::Window { after, before } => (after, Some(before)),
            ReplayQuery::All => (0, None),
        };
        let first_wanted = after.saturating_add(1);
        let floor = self.floor();
        let asks_anything = before.is_none_or(|before| first_wanted < before);
        let truncated = (asks_anything && first_wanted < floor).then_some(floor);
        let events = self
            .events
            .iter()
            .map(|(event, _)| event)
            .filter(|event| event.seq > after && before.is_none_or(|before| event.seq < before))
            .cloned()
            .collect();
        Replay { truncated, events }
    }
}

fn event_size(data: &RunEventData) -> usize {
    let payload = match data {
        RunEventData::Out(bytes) | RunEventData::Err(bytes) => bytes.len(),
        other => other.to_json().as_ref().map_or(0, String::len),
    };
    payload + EVENT_OVERHEAD
}

/// One numbered input frame (§4.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputFrame {
    /// The frame's number, from 1.
    pub seq: u64,
    /// The input bytes.
    pub bytes: Vec<u8>,
    /// Whether input ends after this frame.
    pub close: bool,
}

/// How an input frame was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOutcome {
    /// The next frame in order; its bytes go to the run.
    Accepted,
    /// A repeat of an accepted frame with identical content; no effect.
    Repeated,
}

/// Why an input frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputRefusal {
    /// The frame skips ahead of the next expected `seq`.
    Gap {
        /// The `seq` the session expects next.
        expected: u64,
    },
    /// The frame repeats an accepted `seq` with different content.
    Conflict,
    /// Input was already closed by an earlier frame.
    Closed,
}

impl InputRefusal {
    /// The refusal's `kind` on the wire.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Gap { .. } => "gap",
            Self::Conflict => "conflict",
            Self::Closed => "closed",
        }
    }
}

/// The in-order acceptance rule for one session's input frames (§4.2).
#[derive(Clone, Debug, Default)]
pub struct InputFrames {
    digests: Vec<[u8; 32]>,
    closed: bool,
}

impl InputFrames {
    /// The `seq` the next new frame must carry.
    #[must_use]
    pub fn next_seq(&self) -> u64 {
        u64::try_from(self.digests.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1)
    }

    /// Whether a frame has closed input.
    #[must_use]
    pub const fn closed(&self) -> bool {
        self.closed
    }

    /// Take or refuse a frame.
    ///
    /// # Errors
    ///
    /// Refuses a frame past the next expected `seq` (or `seq` 0) as a gap,
    /// a repeat with different content as a conflict, and a new frame after
    /// input closed.
    pub fn offer(&mut self, frame: &InputFrame) -> Result<InputOutcome, InputRefusal> {
        let digest = frame_digest(frame);
        let next = self.next_seq();
        if frame.seq == 0 || frame.seq > next {
            return Err(InputRefusal::Gap { expected: next });
        }
        if frame.seq < next {
            let Ok(index) = usize::try_from(frame.seq - 1) else {
                return Err(InputRefusal::Conflict);
            };
            return if self.digests.get(index) == Some(&digest) {
                Ok(InputOutcome::Repeated)
            } else {
                Err(InputRefusal::Conflict)
            };
        }
        if self.closed {
            return Err(InputRefusal::Closed);
        }
        self.digests.push(digest);
        self.closed = frame.close;
        Ok(InputOutcome::Accepted)
    }
}

fn frame_digest(frame: &InputFrame) -> [u8; 32] {
    Sha256::new()
        .chain_update([u8::from(frame.close)])
        .chain_update(&frame.bytes)
        .finalize()
        .into()
}

/// A session's input side.
#[derive(Debug, Default)]
struct SessionInput {
    frames: InputFrames,
    /// The running run's stdin, while it takes session input.
    sender: Option<mpsc::Sender<Vec<u8>>>,
    /// Whether a run taking session input was attached.
    attached: bool,
}

/// Whether a session has its run.
#[derive(Debug, Default)]
enum RunSlot {
    /// No run attached yet.
    #[default]
    Empty,
    /// A run is being started.
    Starting,
    /// The run's task.
    Running(tokio::task::AbortHandle),
}

/// One run session (§4.3).
#[derive(Debug)]
pub struct Session {
    id: String,
    log: Mutex<EventLog>,
    updates: watch::Sender<u64>,
    input: tokio::sync::Mutex<SessionInput>,
    last_active: Mutex<Instant>,
    run: Mutex<RunSlot>,
}

impl Session {
    fn new(id: String, byte_cap: usize) -> Self {
        Self {
            id,
            log: Mutex::new(EventLog::new(byte_cap)),
            updates: watch::Sender::new(0),
            input: tokio::sync::Mutex::new(SessionInput::default()),
            last_active: Mutex::new(Instant::now()),
            run: Mutex::new(RunSlot::Empty),
        }
    }

    /// The session id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Append an event to the log and wake attached readers.
    pub fn push(&self, data: RunEventData) -> u64 {
        let seq = self
            .log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data);
        self.touch();
        self.updates.send_replace(seq);
        seq
    }

    /// Answer a replay query, with whether the run has finished.
    #[must_use]
    pub fn replay(&self, query: ReplayQuery) -> (Replay, bool) {
        let log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
        (log.replay(query), log.finished())
    }

    fn touch(&self) {
        *self
            .last_active
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn idle_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(
            *self
                .last_active
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Offer an input frame; accepted bytes go to the run's stdin, and a
    /// closing frame ends it.
    ///
    /// # Errors
    ///
    /// The frame's refusal, or `None` when the session has no run taking
    /// session input.
    pub async fn input(&self, frame: InputFrame) -> Result<InputOutcome, Option<InputRefusal>> {
        self.touch();
        let mut input = self.input.lock().await;
        if !input.attached {
            return Err(None);
        }
        let outcome = input.frames.offer(&frame).map_err(Some)?;
        if outcome == InputOutcome::Accepted {
            if let Some(sender) = &input.sender {
                // A run that has ended no longer reads input; the frame is
                // still taken, in order.
                let _ = sender.send(frame.bytes).await;
            }
            if frame.close {
                input.sender = None;
            }
        }
        Ok(outcome)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let RunSlot::Running(run) = self.run.get_mut().unwrap_or_else(PoisonError::into_inner) {
            run.abort();
        }
    }
}

/// The sessions a server holds, bounded by an idle TTL (§4.4).
#[derive(Debug)]
pub struct SessionStore {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    byte_cap: usize,
    idle_ttl: Duration,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new(DEFAULT_SESSION_BYTE_CAP, DEFAULT_SESSION_IDLE_TTL)
    }
}

impl SessionStore {
    /// A store whose logs keep `byte_cap` bytes and whose sessions are
    /// dropped after `idle_ttl` without activity (at most
    /// [`MAX_SESSION_IDLE_TTL`]).
    #[must_use]
    pub fn new(byte_cap: usize, idle_ttl: Duration) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            byte_cap,
            idle_ttl: idle_ttl.min(MAX_SESSION_IDLE_TTL),
        }
    }

    /// Create a session with a new random id.
    #[must_use]
    pub fn create(&self) -> Arc<Session> {
        let mut id = [0_u8; 16];
        rand::fill(&mut id);
        let id = id.iter().fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
        let session = Arc::new(Session::new(id.clone(), self.byte_cap));
        let mut sessions = self.lock();
        sweep(&mut sessions, self.idle_ttl);
        sessions.insert(id, Arc::clone(&session));
        session
    }

    /// The session with `id`, if it has not expired.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        let mut sessions = self.lock();
        sweep(&mut sessions, self.idle_ttl);
        let session = sessions.get(id).cloned()?;
        session.touch();
        Some(session)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn sweep(sessions: &mut HashMap<String, Arc<Session>>, idle_ttl: Duration) {
    let now = Instant::now();
    sessions.retain(|_, session| session.idle_for(now) <= idle_ttl);
}

/// The session routes, served with the runs router's state.
pub(crate) fn session_routes() -> Router<Arc<RunsConfig>> {
    Router::new()
        .route(SESSIONS_PATH, post(create_handler))
        .route(SESSION_INPUT_PATH, post(input_handler))
        .route(SESSION_EVENTS_PATH, get(events_handler))
        .route(SESSION_SOCKET_PATH, get(socket_handler))
}

fn refuse_non_local() -> Response {
    error_response(
        StatusCode::FORBIDDEN,
        "sessions are served to loopback clients only (#10392 §4.7)",
    )
}

fn unknown_session(id: &str) -> Response {
    error_response(StatusCode::NOT_FOUND, format!("unknown session {id:?}"))
}

async fn create_handler(
    State(config): State<Arc<RunsConfig>>,
    ConnectInfoPeer(peer): ConnectInfoPeer,
    headers: HeaderMap,
) -> Response {
    if !peer_allowed(peer, &headers) {
        return refuse_non_local();
    }
    let session = config.sessions.create();
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": session.id() })),
    )
        .into_response()
}

/// Start a run in a session and answer with the session's events (§4.3).
/// Called by the runs handler for a streamed run that needs a session.
pub(crate) async fn start_session_run(
    config: &RunsConfig,
    request: &RunRequest,
    stdin: Option<Vec<u8>>,
    grant: Option<&AppGrant>,
) -> Response {
    let session = match &request.session {
        Some(id) => match config.sessions.get(id) {
            Some(session) => session,
            None => return unknown_session(id),
        },
        None => config.sessions.create(),
    };
    {
        let mut run = session.run.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(*run, RunSlot::Empty) {
            return error_response(
                StatusCode::CONFLICT,
                format!("session {:?} already has a run", session.id()),
            );
        }
        // Reserve the slot so a second run cannot attach while this starts.
        *run = RunSlot::Starting;
    }
    let feed = if request.stdin.mode == StdinMode::Session {
        let (sender, frames) = mpsc::channel(EVENT_BUFFER);
        let mut input = session.input.lock().await;
        input.sender = Some(sender);
        input.attached = true;
        StdinFeed::Frames(frames)
    } else {
        StdinFeed::from(stdin)
    };
    let started = match start_run(config, request, feed, grant) {
        Ok(started) => started,
        Err(err) => {
            *session.run.lock().unwrap_or_else(PoisonError::into_inner) = RunSlot::Empty;
            tracing::warn!(error = %err, "session run could not be started");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("run could not be started: {err}"),
            );
        }
    };
    session.push(RunEventData::Session(session.id().to_owned()));
    let (events, mut received) = mpsc::channel::<RunEvent>(EVENT_BUFFER);
    let writer = Arc::clone(&session);
    let task = tokio::spawn(async move {
        let record = async {
            while let Some(event) = received.recv().await {
                writer.push(event.data);
            }
        };
        let (streamed, ()) = tokio::join!(started.stream(events), record);
        if let Err(err) = streamed {
            tracing::warn!(error = %err, "session run stream failed");
        }
    });
    *session.run.lock().unwrap_or_else(PoisonError::into_inner) =
        RunSlot::Running(task.abort_handle());
    sse_response(session, ReplayQuery::Since(0), true)
}

/// The query string of an attach request (§4.4).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsParams {
    since: Option<u64>,
    after: Option<u64>,
    before: Option<u64>,
    all: Option<u8>,
}

/// Turn attach parameters into a query and whether to follow live.
fn replay_query(
    params: &EventsParams,
    last_event_id: Option<u64>,
) -> Result<(ReplayQuery, bool), String> {
    let cursor = match (params.since, params.after) {
        (Some(since), Some(after)) if since != after => {
            return Err("since and after name the same cursor; give one".to_owned());
        }
        (since, after) => since.or(after),
    };
    if params.all == Some(1) {
        if cursor.is_some() || params.before.is_some() {
            return Err("all=1 takes no other cursor".to_owned());
        }
        return Ok((ReplayQuery::All, true));
    }
    match params.before {
        Some(before) => Ok((
            ReplayQuery::Window {
                after: cursor.unwrap_or(0),
                before,
            },
            false,
        )),
        None => Ok((
            ReplayQuery::Since(cursor.or(last_event_id).unwrap_or(0)),
            true,
        )),
    }
}

async fn events_handler(
    State(config): State<Arc<RunsConfig>>,
    Path(id): Path<String>,
    ConnectInfoPeer(peer): ConnectInfoPeer,
    headers: HeaderMap,
    params: Result<Query<EventsParams>, QueryRejection>,
) -> Response {
    if !peer_allowed(peer, &headers) {
        return refuse_non_local();
    }
    let Ok(Query(params)) = params else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "query takes since, after, before and all (all=1)",
        );
    };
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok());
    let (query, follow) = match replay_query(&params, last_event_id) {
        Ok(query) => query,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };
    let Some(session) = config.sessions.get(&id) else {
        return unknown_session(&id);
    };
    sse_response(session, query, follow)
}

/// What an attached reader receives.
#[derive(Debug)]
enum Outgoing {
    /// Events below this first retained `seq` were dropped.
    Truncated(u64),
    /// A logged event.
    Event(RunEvent),
}

// The session is owned so the spawned reader keeps it alive.
#[allow(clippy::needless_pass_by_value)]
/// Feed `out` with the events `query` asks for and, when `follow`, the
/// events that arrive after them until the run's `exit` event.
async fn attach(
    session: Arc<Session>,
    query: ReplayQuery,
    follow: bool,
    out: mpsc::Sender<Outgoing>,
) {
    let mut updates = session.updates.subscribe();
    let mut query = query;
    loop {
        updates.mark_unchanged();
        let (replay, finished) = session.replay(query);
        if let Some(floor) = replay.truncated
            && out.send(Outgoing::Truncated(floor)).await.is_err()
        {
            return;
        }
        let mut cursor = match query {
            ReplayQuery::Since(since) => since,
            ReplayQuery::Window { after, .. } => after,
            ReplayQuery::All => 0,
        };
        for event in replay.events {
            cursor = event.seq;
            if out.send(Outgoing::Event(event)).await.is_err() {
                return;
            }
        }
        if !follow || finished || updates.changed().await.is_err() {
            return;
        }
        query = ReplayQuery::Since(cursor);
    }
}

fn truncated_json(floor: u64) -> String {
    serde_json::json!({ "truncated": true, "first_retained_seq": floor }).to_string()
}

fn sse_response(session: Arc<Session>, query: ReplayQuery, follow: bool) -> Response {
    let (out, received) = mpsc::channel(EVENT_BUFFER);
    tokio::spawn(attach(session, query, follow, out));
    let stream = futures::stream::unfold(received, |mut received| async move {
        let event = match received.recv().await? {
            Outgoing::Truncated(floor) => Event::default().event("app").data(truncated_json(floor)),
            Outgoing::Event(event) => sse_event(&event),
        };
        Some((Ok::<_, std::convert::Infallible>(event), received))
    });
    Sse::new(stream).into_response()
}

/// An input frame as it is sent (§4.2).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFrameBody {
    seq: u64,
    /// The input bytes, base64.
    #[serde(default)]
    bytes: String,
    #[serde(default)]
    close: bool,
}

/// The answer to an input frame, over HTTP or the socket.
#[derive(Debug, Serialize)]
struct InputAnswer {
    seq: u64,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl InputAnswer {
    const fn refused(seq: u64, kind: &'static str, error: String) -> Self {
        Self {
            seq,
            status: "refused",
            kind: Some(kind),
            expected: None,
            error: Some(error),
        }
    }
}

/// Take one input frame and say what became of it.
async fn take_input(session: &Session, body: InputFrameBody) -> (StatusCode, InputAnswer) {
    let bytes = match base64::engine::general_purpose::STANDARD.decode(&body.bytes) {
        Ok(bytes) => bytes,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                InputAnswer::refused(body.seq, "invalid", format!("bytes is not base64: {err}")),
            );
        }
    };
    let frame = InputFrame {
        seq: body.seq,
        bytes,
        close: body.close,
    };
    match session.input(frame).await {
        Ok(outcome) => {
            let (status, word) = match outcome {
                InputOutcome::Accepted => (StatusCode::ACCEPTED, "accepted"),
                InputOutcome::Repeated => (StatusCode::OK, "repeated"),
            };
            (
                status,
                InputAnswer {
                    seq: body.seq,
                    status: word,
                    kind: None,
                    expected: None,
                    error: None,
                },
            )
        }
        Err(None) => (
            StatusCode::CONFLICT,
            InputAnswer::refused(
                body.seq,
                "no_input",
                "this session has no run taking session input".to_owned(),
            ),
        ),
        Err(Some(refusal)) => {
            let mut answer = InputAnswer::refused(
                body.seq,
                refusal.kind(),
                match refusal {
                    InputRefusal::Gap { expected } => {
                        format!("frame {} skips ahead; expected {expected}", body.seq)
                    }
                    InputRefusal::Conflict => {
                        format!("frame {} was already taken with other content", body.seq)
                    }
                    InputRefusal::Closed => "input is closed".to_owned(),
                },
            );
            if let InputRefusal::Gap { expected } = refusal {
                answer.expected = Some(expected);
            }
            (StatusCode::CONFLICT, answer)
        }
    }
}

async fn input_handler(
    State(config): State<Arc<RunsConfig>>,
    Path(id): Path<String>,
    ConnectInfoPeer(peer): ConnectInfoPeer,
    headers: HeaderMap,
    body: Result<Json<InputFrameBody>, JsonRejection>,
) -> Response {
    if !peer_allowed(peer, &headers) {
        return refuse_non_local();
    }
    let Some(session) = config.sessions.get(&id) else {
        return unknown_session(&id);
    };
    let Json(body) = match body {
        Ok(body) => body,
        Err(rejection) => return rejection.into_response(),
    };
    let (status, answer) = take_input(&session, body).await;
    (status, Json(answer)).into_response()
}

async fn socket_handler(
    State(config): State<Arc<RunsConfig>>,
    Path(id): Path<String>,
    ConnectInfoPeer(peer): ConnectInfoPeer,
    headers: HeaderMap,
    params: Result<Query<EventsParams>, QueryRejection>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !peer_allowed(peer, &headers) {
        return refuse_non_local();
    }
    let Ok(Query(params)) = params else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "query takes since, after, before and all (all=1)",
        );
    };
    let (query, follow) = match replay_query(&params, None) {
        Ok(query) => query,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };
    let Some(session) = config.sessions.get(&id) else {
        return unknown_session(&id);
    };
    upgrade.on_upgrade(move |socket| serve_socket(socket, session, query, follow))
}

/// An event as the socket carries it: the SSE event's id, name and data.
fn socket_event(event: &RunEvent) -> String {
    let data: serde_json::Value = event
        .data
        .to_json()
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({ "id": event.seq, "event": event.data.name(), "data": data }).to_string()
}

// The session is owned so the socket keeps it alive.
#[allow(clippy::needless_pass_by_value)]
async fn serve_socket(
    mut socket: WebSocket,
    session: Arc<Session>,
    query: ReplayQuery,
    follow: bool,
) {
    let (out, mut received) = mpsc::channel(EVENT_BUFFER);
    let reader = tokio::spawn(attach(Arc::clone(&session), query, follow, out));
    let mut events_open = true;
    loop {
        tokio::select! {
            outgoing = received.recv(), if events_open => {
                let text = match outgoing {
                    Some(Outgoing::Event(event)) => socket_event(&event),
                    Some(Outgoing::Truncated(floor)) => serde_json::json!({
                        "event": "app",
                        "data": { "truncated": true, "first_retained_seq": floor },
                    })
                    .to_string(),
                    None => {
                        events_open = false;
                        continue;
                    }
                };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                let answer = match serde_json::from_str::<InputFrameBody>(text.as_str()) {
                    Ok(body) => take_input(&session, body).await.1,
                    Err(err) => InputAnswer::refused(0, "invalid", format!("not an input frame: {err}")),
                };
                let text = serde_json::to_string(&answer).unwrap_or_default();
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    }
    reader.abort();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn chunk(size: usize) -> RunEventData {
        RunEventData::Out(vec![b'x'; size])
    }

    fn log_of(sizes: &[usize], cap: usize) -> EventLog {
        let mut log = EventLog::new(cap);
        for size in sizes {
            log.push(chunk(*size));
        }
        log
    }

    #[test]
    fn replay_modes_select_events() {
        let log = log_of(&[1, 1, 1, 1], usize::MAX);
        let seqs = |replay: Replay| replay.events.iter().map(|e| e.seq).collect::<Vec<_>>();
        assert_eq!(seqs(log.replay(ReplayQuery::Since(2))), [3, 4]);
        assert_eq!(seqs(log.replay(ReplayQuery::All)), [1, 2, 3, 4]);
        assert_eq!(
            seqs(log.replay(ReplayQuery::Window {
                after: 1,
                before: 4
            })),
            [2, 3]
        );
        assert_eq!(log.replay(ReplayQuery::Since(0)).truncated, None);
    }

    #[test]
    fn retention_drops_oldest_and_marks_the_floor() {
        let log = log_of(&[10, 10, 10, 10], 3 * (10 + EVENT_OVERHEAD));
        assert_eq!(log.floor(), 2);
        assert_eq!(log.replay(ReplayQuery::Since(0)).truncated, Some(2));
        assert_eq!(log.replay(ReplayQuery::All).truncated, Some(2));
        assert_eq!(log.replay(ReplayQuery::Since(1)).truncated, None);
        let empty_window = log.replay(ReplayQuery::Window {
            after: 0,
            before: 1,
        });
        assert_eq!(empty_window.truncated, None);
        assert_eq!(empty_window.events, [] as [RunEvent; 0]);
    }

    proptest! {
        /// §7 replay invariant: for any split point `k`, the window up to
        /// `k` followed by `since=k` is the full retained log, and the
        /// truncation marker shows whenever a cursor is below the floor.
        #[test]
        fn replay_is_a_pure_split(
            sizes in proptest::collection::vec(0_usize..64, 0..40),
            cap in 1_usize..2048,
            split in 0_u64..48,
        ) {
            let log = log_of(&sizes, cap);
            let full = log.replay(ReplayQuery::All);
            let k = split.min(log.last_seq());
            let mut joined = log.replay(ReplayQuery::Window { after: 0, before: k + 1 }).events;
            joined.extend(log.replay(ReplayQuery::Since(k)).events);
            prop_assert_eq!(&joined, &full.events);
            let seqs: Vec<u64> = full.events.iter().map(|e| e.seq).collect();
            let expected: Vec<u64> = (log.floor()..=log.last_seq()).collect();
            prop_assert_eq!(seqs, expected);
            let below_floor = k + 1 < log.floor();
            prop_assert_eq!(
                log.replay(ReplayQuery::Since(k)).truncated.is_some(),
                below_floor
            );
        }

        /// §7 input ordering: random frame posts match a model that
        /// accepts in order and refuses gaps and conflicting repeats.
        #[test]
        fn input_frames_match_the_in_order_model(
            posts in proptest::collection::vec((0_u64..8, 0_u8..3, any::<bool>()), 0..60),
        ) {
            let mut frames = InputFrames::default();
            let mut model: Vec<(u8, bool)> = Vec::new();
            for (seq, content, close) in posts {
                let frame = InputFrame { seq, bytes: vec![content], close };
                let got = frames.offer(&frame);
                let next = u64::try_from(model.len()).unwrap() + 1;
                let closed = model.last().is_some_and(|(_, close)| *close);
                let want = if seq == 0 || seq > next {
                    Err(InputRefusal::Gap { expected: next })
                } else if seq < next {
                    let index = usize::try_from(seq - 1).unwrap();
                    if model[index] == (content, close) {
                        Ok(InputOutcome::Repeated)
                    } else {
                        Err(InputRefusal::Conflict)
                    }
                } else if closed {
                    Err(InputRefusal::Closed)
                } else {
                    model.push((content, close));
                    Ok(InputOutcome::Accepted)
                };
                prop_assert_eq!(got, want);
            }
            prop_assert_eq!(frames.next_seq(), u64::try_from(model.len()).unwrap() + 1);
        }
    }

    #[test]
    fn attach_parameters_map_to_queries() {
        let params = |since, after, before, all| EventsParams {
            since,
            after,
            before,
            all,
        };
        assert_eq!(
            replay_query(&params(None, None, None, None), Some(7)),
            Ok((ReplayQuery::Since(7), true))
        );
        assert_eq!(
            replay_query(&params(Some(3), None, None, None), Some(7)),
            Ok((ReplayQuery::Since(3), true))
        );
        assert_eq!(
            replay_query(&params(None, Some(2), Some(5), None), None),
            Ok((
                ReplayQuery::Window {
                    after: 2,
                    before: 5
                },
                false
            ))
        );
        assert_eq!(
            replay_query(&params(None, None, None, Some(1)), None),
            Ok((ReplayQuery::All, true))
        );
        assert!(replay_query(&params(Some(1), Some(2), None, None), None).is_err());
    }

    #[test]
    fn stores_expire_idle_sessions() {
        let store = SessionStore::new(1024, Duration::ZERO);
        let session = store.create();
        let id = session.id().to_owned();
        drop(session);
        std::thread::sleep(Duration::from_millis(5));
        assert!(store.get(&id).is_none());
        let fresh = SessionStore::default();
        let session = fresh.create();
        assert_eq!(fresh.get(session.id()).unwrap().id(), session.id());
    }

    mod http {
        use super::super::*;
        use crate::runs::{RUNS_PATH, runs_routes};
        use axum::body::Body;
        use axum::extract::ConnectInfo;
        use axum::http::Request as HttpRequest;
        use futures::StreamExt as _;
        use std::net::SocketAddr;
        use std::path::PathBuf;
        use tower::ServiceExt as _;

        fn app(config: RunsConfig) -> Router {
            runs_routes(config)
        }

        fn local(mut request: HttpRequest<Body>) -> HttpRequest<Body> {
            request
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40_000))));
            request
        }

        fn post_json(path: &str, body: &serde_json::Value, events: bool) -> HttpRequest<Body> {
            let mut builder = HttpRequest::post(path).header("content-type", "application/json");
            if events {
                builder = builder.header("accept", "text/event-stream");
            }
            local(builder.body(Body::from(body.to_string())).unwrap())
        }

        async fn json_of(response: Response) -> (StatusCode, serde_json::Value) {
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, serde_json::from_slice(&bytes).unwrap_or_default())
        }

        /// `(id, event, data)` of each SSE block; the marker has no id.
        fn parse(text: &str) -> Vec<(Option<u64>, String, serde_json::Value)> {
            text.split("\n\n")
                .filter(|block| !block.trim().is_empty())
                .map(|block| {
                    let mut id = None;
                    let mut event = String::new();
                    let mut data = String::new();
                    for line in block.lines() {
                        if let Some(value) = line.strip_prefix("id:") {
                            id = Some(value.trim().parse().unwrap());
                        } else if let Some(value) = line.strip_prefix("event:") {
                            value.trim().clone_into(&mut event);
                        } else if let Some(value) = line.strip_prefix("data:") {
                            data.push_str(value.trim_start());
                        }
                    }
                    (id, event, serde_json::from_str(&data).unwrap())
                })
                .collect()
        }

        fn out_text(events: &[(Option<u64>, String, serde_json::Value)]) -> String {
            events
                .iter()
                .filter(|(_, event, _)| event == "out")
                .map(|(_, _, data)| data["data"].as_str().unwrap().to_owned())
                .collect()
        }

        async fn body_text(response: Response) -> String {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        }

        fn frame(seq: u64, text: &str, close: bool) -> serde_json::Value {
            serde_json::json!({
                "seq": seq,
                "bytes": base64::engine::general_purpose::STANDARD.encode(text),
                "close": close,
            })
        }

        #[tokio::test]
        async fn session_runs_take_input_frames_until_close() {
            let router = app(RunsConfig::new(PathBuf::from("sh")));
            let response = router
                .clone()
                .oneshot(post_json(
                    RUNS_PATH,
                    &serde_json::json!({
                        "argv": ["-c", "cat; echo end"],
                        "stdin": { "mode": "session" }
                    }),
                    true,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let mut stream = response.into_body().into_data_stream();
            let mut text = String::new();
            while !text.contains("\n\n") {
                let chunk = stream.next().await.unwrap().unwrap();
                text.push_str(std::str::from_utf8(&chunk).unwrap());
            }
            let first = parse(&text);
            let (id, event, data) = &first[0];
            assert_eq!((*id, event.as_str()), (Some(1), "app"));
            let session = data["session"].as_str().unwrap().to_owned();
            let input = format!("/v1/sessions/{session}/input");

            let post = |body: serde_json::Value| {
                let router = router.clone();
                let input = input.clone();
                async move {
                    json_of(
                        router
                            .oneshot(post_json(&input, &body, false))
                            .await
                            .unwrap(),
                    )
                    .await
                }
            };
            assert_eq!(
                post(frame(1, "hello ", false)).await.0,
                StatusCode::ACCEPTED
            );
            let (status, gap) = post(frame(3, "x", false)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(gap["kind"], "gap");
            assert_eq!(gap["expected"], 2);
            let (status, repeat) = post(frame(1, "hello ", false)).await;
            assert_eq!(
                (status, repeat["status"].as_str()),
                (StatusCode::OK, Some("repeated"))
            );
            let (status, conflict) = post(frame(1, "other", false)).await;
            assert_eq!(
                (status, conflict["kind"].as_str()),
                (StatusCode::CONFLICT, Some("conflict"))
            );
            assert_eq!(
                post(frame(2, "world\n", true)).await.0,
                StatusCode::ACCEPTED
            );
            let (_, closed) = post(frame(3, "late", false)).await;
            assert_eq!(closed["kind"], "closed");

            while let Some(chunk) = stream.next().await {
                text.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
            }
            let events = parse(&text);
            assert_eq!(out_text(&events), "hello world\nend\n");
            let (_, last, exit) = events.last().unwrap();
            assert_eq!(last, "exit");
            assert_eq!(exit["exit_code"], 0);

            // Replay: since=1 skips the session event; a window reads a slice.
            let events_path = format!("/v1/sessions/{session}/events");
            let get = |query: &str| {
                let router = router.clone();
                let uri = format!("{events_path}{query}");
                async move {
                    let request = local(HttpRequest::get(uri).body(Body::empty()).unwrap());
                    body_text(router.oneshot(request).await.unwrap()).await
                }
            };
            let replayed = parse(&get("?since=1").await);
            assert_eq!(replayed.first().unwrap().0, Some(2));
            assert_eq!(out_text(&replayed), "hello world\nend\n");
            let all = parse(&get("?all=1").await);
            assert_eq!(all.len(), events.len());
            let window = parse(&get("?after=0&before=2").await);
            assert_eq!(window.len(), 1);
            assert_eq!(window[0].2["session"], session.as_str());
        }

        #[tokio::test]
        async fn created_sessions_answer_before_output_and_replay_with_a_marker() {
            let mut config = RunsConfig::new(PathBuf::from("sh"));
            config.sessions = Arc::new(SessionStore::new(
                3 * (EVENT_OVERHEAD + 2),
                DEFAULT_SESSION_IDLE_TTL,
            ));
            let router = app(config);
            let (status, created) = json_of(
                router
                    .clone()
                    .oneshot(post_json(SESSIONS_PATH, &serde_json::json!({}), false))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            let session = created["id"].as_str().unwrap().to_owned();
            let response = router
                .clone()
                .oneshot(post_json(
                    RUNS_PATH,
                    &serde_json::json!({
                        "argv": ["-c", "for n in 1 2 3 4 5 6; do printf '%s\\n' $n; sleep 0.05; done"],
                        "session": session,
                    }),
                    true,
                ))
                .await
                .unwrap();
            let events = parse(&body_text(response).await);
            assert_eq!(events.last().unwrap().1, "exit");

            let uri = format!("/v1/sessions/{session}/events?since=0");
            let request = local(HttpRequest::get(uri).body(Body::empty()).unwrap());
            let replayed = parse(&body_text(router.clone().oneshot(request).await.unwrap()).await);
            let (id, event, marker) = &replayed[0];
            assert_eq!((*id, event.as_str()), (None, "app"));
            assert_eq!(marker["truncated"], true);
            let floor = marker["first_retained_seq"].as_u64().unwrap();
            assert!(floor > 1);
            assert_eq!(replayed[1].0, Some(floor));
            assert_eq!(replayed.last().unwrap().1, "exit");

            // A second run in the same session is refused.
            let again = router
                .oneshot(post_json(
                    RUNS_PATH,
                    &serde_json::json!({ "argv": ["-c", "true"], "session": session }),
                    true,
                ))
                .await
                .unwrap();
            assert_eq!(again.status(), StatusCode::CONFLICT);
        }

        #[tokio::test]
        async fn session_routes_refuse_what_they_cannot_serve() {
            let router = app(RunsConfig::new(PathBuf::from("sh")));
            let plain = router
                .clone()
                .oneshot(post_json(
                    RUNS_PATH,
                    &serde_json::json!({ "argv": [], "stdin": { "mode": "session" } }),
                    false,
                ))
                .await
                .unwrap();
            assert_eq!(plain.status(), StatusCode::NOT_ACCEPTABLE);
            let unknown = router
                .clone()
                .oneshot(post_json(
                    "/v1/sessions/nope/input",
                    &frame(1, "x", false),
                    false,
                ))
                .await
                .unwrap();
            assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
            let (_, created) = json_of(
                router
                    .clone()
                    .oneshot(post_json(SESSIONS_PATH, &serde_json::json!({}), false))
                    .await
                    .unwrap(),
            )
            .await;
            let input = format!("/v1/sessions/{}/input", created["id"].as_str().unwrap());
            let (status, no_input) = json_of(
                router
                    .clone()
                    .oneshot(post_json(&input, &frame(1, "x", false), false))
                    .await
                    .unwrap(),
            )
            .await;
            assert_eq!(
                (status, no_input["kind"].as_str()),
                (StatusCode::CONFLICT, Some("no_input"))
            );
            let mut remote = HttpRequest::post(SESSIONS_PATH)
                .body(Body::empty())
                .unwrap();
            remote
                .extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 7], 40_000))));
            assert_eq!(
                router.oneshot(remote).await.unwrap().status(),
                StatusCode::FORBIDDEN
            );
        }
    }
}
