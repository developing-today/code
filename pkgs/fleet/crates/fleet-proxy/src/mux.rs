//! Proxy-side tunnel mux — seam record §7 step 2 for issue #5756
//! (`doc/2026-11-16T10-40-00Z_design_id5756_multiplexed_streams_seam.md`).
//!
//! This module is the proxy half of the `proxy-mux` frame family
//! whose wire contract landed in `fleet_proxy_proto::mux` (step
//! 1, PR #13330): a stream table, frame interleaving by stream id
//! over one persistent connection, application-level keepalive,
//! and the failure semantics the record fixes in §4:
//!
//! * a dead tunnel fails every in-flight stream immediately with
//!   a tunnel-lost error that preserves the stream's trip id (the
//!   ≤2 s bound of PR #6490 — detection here is the transport's
//!   own EOF/error, or the keepalive liveness horizon for a
//!   silently half-open peer);
//! * reconnect is single-flight: the fail-all pass runs exactly
//!   once however many threads notice the death, and the tunnel
//!   opens the dispatch path performs ride the broker's existing
//!   single-flight recovery gate (PR #6490) in `serve` — a proxy
//!   wobble cannot stampede opens the way the pool of 50 did;
//! * backpressure is per-stream credit ([`StreamWindow`]): a
//!   sender never exceeds granted credit, so a slow consumer
//!   stalls its own stream, never the tunnel and never other
//!   streams.
//!
//! The endpoint is symmetric machinery used asymmetrically, as
//! the record's endpoint table (§3) fixes: the proxy *terminates*
//! streams (clients originate them — the client call path is
//! step 3, the laptop broker's stream dispatch is step 4).
//! [`MuxSession::open_stream`] exists so the client call path
//! (`client::mux_roundtrip`, step 3) and tests can originate
//! against the same table/interleaving code the proxy
//! terminates with.
//!
//! Nothing here changes the pooled path: `serve` advertises
//! [`fleet_proxy_proto::mux::CAP_PROXY_MUX`] now that this code
//! has landed, and a session enters mux mode only when the
//! client's hello advertised it back
//! ([`fleet_proxy_proto::mux::mux_negotiated`]) — the record's
//! no-one-sided-shipment rule (§5). Absence on either side leaves
//! the pooled HTTP-per-call path byte-for-byte in charge.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fleet_proxy_proto::mux::{
    MUX_KEEPALIVE_S, OpenIntent, RST_CANCELLED, RST_PROTOCOL_ERROR, RST_TIMEOUT, StreamCeiling,
    StreamFrame, StreamId, StreamWindow,
};
use fleet_proxy_proto::{ProtoError, read_frame, write_frame};

/// Session tuning for a mux endpoint. Production uses
/// [`MuxConfig::default`]: the record's keepalive constant
/// ([`MUX_KEEPALIVE_S`]), a liveness horizon of two keepalive
/// intervals without any inbound frame, and the proto crate's
/// default per-stream ceiling. Tests shrink all three.
#[derive(Debug, Clone)]
pub struct MuxConfig {
    /// Interval between keepalive `ping` frames on an idle
    /// tunnel.
    pub keepalive: Duration,
    /// No inbound frame for this long ⇒ the tunnel is declared
    /// dead even though the transport has not errored (a
    /// half-open peer). Transport EOF/error is detected
    /// immediately and never waits for this horizon.
    pub liveness_timeout: Duration,
    /// Per-stream ceiling: the longest a stream worker waits for
    /// the peer before the stream is reset as timed out.
    pub ceiling: StreamCeiling,
}

impl Default for MuxConfig {
    fn default() -> Self {
        let keepalive = Duration::from_secs(MUX_KEEPALIVE_S);
        MuxConfig {
            keepalive,
            liveness_timeout: keepalive * 2,
            ceiling: StreamCeiling::default(),
        }
    }
}

/// How a stream ended abnormally. Every variant is terminal for
/// the stream; none of them ends the tunnel except
/// [`StreamError::TunnelLost`], which ends every stream at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamError {
    /// The tunnel died under the stream. Carries the stream's
    /// own trip id, preserved across the failure exactly as the
    /// record's acceptance requires (§6 item 4): trip logs on
    /// both sides keep the same id per call, failures included.
    TunnelLost { trip: Option<String> },
    /// The peer reset the stream; `reason` is one of the proto
    /// crate's `RST_*` strings or the peer's own machine-readable
    /// reason.
    Reset { reason: String },
    /// The per-stream ceiling expired waiting for the peer.
    Timeout,
    /// The peer violated the stream contract (frame bound,
    /// window, or id rules) or the stream was used after close.
    Protocol(String),
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StreamError::TunnelLost { trip } => match trip {
                Some(trip) => write!(f, "mux tunnel lost (trip {trip})"),
                None => write!(f, "mux tunnel lost"),
            },
            StreamError::Reset { reason } => write!(f, "mux stream reset: {reason}"),
            StreamError::Timeout => write!(f, "mux stream timed out"),
            StreamError::Protocol(why) => write!(f, "mux stream protocol error: {why}"),
        }
    }
}

impl std::error::Error for StreamError {}

/// One stream's row in the table: the credit window pair, the
/// trip id the failure path must preserve, and the fin bookkeeping
/// that decides when a stream is complete.
#[derive(Debug)]
struct StreamState {
    trip: Option<String>,
    port: u16,
    window: StreamWindow,
    fin_in: bool,
    fin_out: bool,
}

impl StreamState {
    /// Complete: both directions finished. The row is reaped when
    /// the stream's handle drops.
    fn complete(&self) -> bool {
        self.fin_in && self.fin_out
    }
}

/// The stream table — the pure state machine under the session,
/// unit-tested without IO. Oracle: the record §4 rules as the
/// proto crate types them (odd client ids, 64 KiB frame bound,
/// 256 KiB initial window, credit never overrun).
///
/// Rows survive a stream's reset as tombstones in `terminal` so a
/// sender blocked on credit wakes with the real reason instead of
/// a bare "gone"; both maps are reaped by [`StreamTable::reap`]
/// when the stream's handle drops, and wholesale by
/// [`StreamTable::fail_all`].
#[derive(Debug, Default)]
pub struct StreamTable {
    streams: HashMap<StreamId, StreamState>,
    terminal: HashMap<StreamId, String>,
}

impl StreamTable {
    pub fn new() -> StreamTable {
        StreamTable::default()
    }

    /// Register a stream opened by the peer. The id must be a
    /// client-initiated (nonzero, odd) id that is not already in
    /// use — anything else is a protocol violation by the peer.
    pub fn open(
        &mut self,
        id: StreamId,
        port: u16,
        trip: Option<String>,
    ) -> Result<(), ProtoError> {
        if !id.is_client_initiated() {
            return Err(ProtoError::BadFrame(format!(
                "open stream id {} is not a client-initiated (odd) id",
                id.0
            )));
        }
        if self.streams.contains_key(&id) || self.terminal.contains_key(&id) {
            return Err(ProtoError::BadFrame(format!(
                "open stream id {} is already in use",
                id.0
            )));
        }
        self.streams.insert(
            id,
            StreamState {
                trip,
                port,
                window: StreamWindow::new(),
                fin_in: false,
                fin_out: false,
            },
        );
        Ok(())
    }

    /// Register a stream this side originates (the mirror of
    /// [`StreamTable::open`]; ids come from the session's own
    /// odd counter, so only the duplicate rule applies).
    pub fn open_local(
        &mut self,
        id: StreamId,
        port: u16,
        trip: Option<String>,
    ) -> Result<(), ProtoError> {
        if self.streams.contains_key(&id) || self.terminal.contains_key(&id) {
            return Err(ProtoError::BadFrame(format!(
                "open stream id {} is already in use",
                id.0
            )));
        }
        self.streams.insert(
            id,
            StreamState {
                trip,
                port,
                window: StreamWindow::new(),
                fin_in: false,
                fin_out: false,
            },
        );
        Ok(())
    }

    pub fn contains(&self, id: StreamId) -> bool {
        self.streams.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.streams.len()
    }

    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    pub fn trip_of(&self, id: StreamId) -> Option<Option<String>> {
        self.streams.get(&id).map(|s| s.trip.clone())
    }

    pub fn port_of(&self, id: StreamId) -> Option<u16> {
        self.streams.get(&id).map(|s| s.port)
    }

    /// The reset reason recorded for a stream, if it has ended
    /// abnormally.
    pub fn terminal_reason(&self, id: StreamId) -> Option<&str> {
        self.terminal.get(&id).map(|s| s.as_str())
    }

    /// Account for one inbound data chunk against the window this
    /// side granted. An overrun is the peer's protocol violation.
    pub fn note_data_in(&mut self, id: StreamId, len: usize) -> Result<(), ProtoError> {
        let state = self
            .streams
            .get_mut(&id)
            .ok_or_else(|| ProtoError::BadFrame(format!("data for unknown stream {}", id.0)))?;
        state.window.receive(len)
    }

    /// Mark `len` inbound bytes consumed by the stream's worker;
    /// returns the credit to grant back in a `window` frame.
    pub fn note_consumed(&mut self, id: StreamId, len: u64) -> u64 {
        match self.streams.get_mut(&id) {
            Some(state) => state.window.consume(len),
            None => 0,
        }
    }

    /// Account for a `window` frame from the peer: more send
    /// credit for this side.
    pub fn note_window(&mut self, id: StreamId, credit: u64) -> Result<(), ProtoError> {
        let state = self
            .streams
            .get_mut(&id)
            .ok_or_else(|| ProtoError::BadFrame(format!("window for unknown stream {}", id.0)))?;
        state.window.grant(credit);
        Ok(())
    }

    /// Bytes this side may still send on the stream before more
    /// credit arrives (0 for an unknown stream).
    pub fn send_credit(&self, id: StreamId) -> u64 {
        self.streams
            .get(&id)
            .map(|s| s.window.send_credit())
            .unwrap_or(0)
    }

    /// Account for one outbound chunk against granted credit.
    /// Refuses a chunk that would overrun — the caller waits for
    /// a `window` frame instead (see `MuxStream::send_chunk`).
    pub fn note_data_out(&mut self, id: StreamId, len: usize) -> Result<(), ProtoError> {
        let state = self
            .streams
            .get_mut(&id)
            .ok_or_else(|| ProtoError::BadFrame(format!("send on unknown stream {}", id.0)))?;
        state.window.send(len)
    }

    pub fn note_fin_in(&mut self, id: StreamId) {
        if let Some(state) = self.streams.get_mut(&id) {
            state.fin_in = true;
        }
    }

    pub fn note_fin_out(&mut self, id: StreamId) {
        if let Some(state) = self.streams.get_mut(&id) {
            state.fin_out = true;
        }
    }

    pub fn is_complete(&self, id: StreamId) -> bool {
        self.streams.get(&id).map(|s| s.complete()).unwrap_or(false)
    }

    /// Record a reset (either direction): the row leaves the live
    /// table, the reason stays as a tombstone for blocked senders.
    pub fn note_reset(&mut self, id: StreamId, reason: &str) {
        self.streams.remove(&id);
        self.terminal.insert(id, reason.to_string());
    }

    /// Reap a stream entirely (handle dropped): row and tombstone.
    pub fn reap(&mut self, id: StreamId) {
        self.streams.remove(&id);
        self.terminal.remove(&id);
    }

    /// The dead-tunnel pass: every in-flight stream fails at
    /// once. Returns each failed stream's id paired with its
    /// preserved trip id — the caller delivers one tunnel-lost
    /// error per stream, and the trip correlation survives the
    /// failure (record §4, §6 item 4). The table is left empty.
    pub fn fail_all(&mut self) -> Vec<(StreamId, Option<String>)> {
        let failed: Vec<(StreamId, Option<String>)> = self
            .streams
            .drain()
            .map(|(id, state)| (id, state.trip))
            .collect();
        self.terminal.clear();
        failed
    }
}

/// What an `open` asked for, handed to the stream handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenInfo {
    pub stream: StreamId,
    pub port: u16,
    pub intent: OpenIntent,
    pub trip: Option<String>,
}

/// The handler a terminating endpoint runs per opened stream, on
/// its own thread. Originating endpoints pass a handler too —
/// peers can open streams back only by violating the session's
/// direction convention, but the slot keeps the type symmetric.
pub type StreamHandler = Arc<dyn Fn(MuxStream, OpenInfo) + Send + Sync>;

/// Events the reader thread delivers to a stream's worker.
enum StreamEvent {
    Data { bytes: Vec<u8>, status: Option<u16> },
    Fin,
    Rst(String),
    TunnelLost(Option<String>),
}

/// State shared by the session, its reader/pinger threads, and
/// every live stream handle.
struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    table: Mutex<StreamTable>,
    inboxes: Mutex<HashMap<StreamId, std::sync::mpsc::Sender<StreamEvent>>>,
    /// Wakes senders blocked on credit: window frames, resets,
    /// and tunnel death all signal it.
    credit_cv: Condvar,
    dead: AtomicBool,
    last_inbound: Mutex<Instant>,
    next_nonce: AtomicU64,
    next_local_id: AtomicU64,
    config: MuxConfig,
    on_activity: Option<Arc<dyn Fn() + Send + Sync>>,
    close_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Shared {
    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// Write one frame, atomically with respect to other writers:
    /// the lock is held for the whole frame, so frames from many
    /// streams interleave at frame granularity and never tear.
    /// A write failure means the tunnel is gone: the fail-all
    /// pass runs and the caller learns the tunnel is lost.
    fn send_frame(&self, frame: &StreamFrame) -> Result<(), StreamError> {
        if self.is_dead() {
            return Err(StreamError::TunnelLost { trip: None });
        }
        let mut writer = self.writer.lock().unwrap();
        match write_frame(&mut *writer, frame) {
            Ok(()) => Ok(()),
            Err(_) => {
                drop(writer);
                self.kill();
                Err(StreamError::TunnelLost { trip: None })
            }
        }
    }

    /// The fail-all pass, single-shot: the first caller (reader
    /// EOF, writer failure, or the liveness watchdog) flips
    /// `dead` and performs it; every later caller is a no-op.
    /// That single-shot property is the session-level half of
    /// the record's single-flight rule — one death, one pass,
    /// one terminal event per stream, however many threads
    /// notice concurrently.
    fn kill(&self) {
        if self.dead.swap(true, Ordering::SeqCst) {
            return;
        }
        let failed = self.table.lock().unwrap().fail_all();
        let inboxes = std::mem::take(&mut *self.inboxes.lock().unwrap());
        for (id, trip) in failed {
            if let Some(tx) = inboxes.get(&id) {
                let _ = tx.send(StreamEvent::TunnelLost(trip));
            }
        }
        self.credit_cv.notify_all();
        if let Some(hook) = &self.close_hook {
            hook();
        }
    }
}

/// One mux session over one persistent framed connection.
///
/// Construct with the connection's writer half, then call
/// [`MuxSession::run`] with the reader half and the stream
/// handler; `run` blocks until the tunnel dies or the peer goes
/// silent past the liveness horizon. Cloning a session shares
/// the same tunnel (originating streams, tests, shutdown).
#[derive(Clone)]
pub struct MuxSession {
    shared: Arc<Shared>,
}

impl MuxSession {
    /// * `writer` — the connection's write half (a `try_clone` of
    ///   the socket in production).
    /// * `on_activity` — called on every inbound frame; `serve`
    ///   uses it to refresh the broker's idle-exit clock, so a
    ///   broker with a healthy mux session up never idle-exits
    ///   (record §4).
    /// * `close_hook` — called once when the tunnel is declared
    ///   dead; `serve` uses it to shut the socket down, which is
    ///   what unblocks the reader thread of a silently half-open
    ///   peer.
    pub fn new(
        writer: Box<dyn Write + Send>,
        config: MuxConfig,
        on_activity: Option<Arc<dyn Fn() + Send + Sync>>,
        close_hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> MuxSession {
        MuxSession {
            shared: Arc::new(Shared {
                writer: Mutex::new(writer),
                table: Mutex::new(StreamTable::new()),
                inboxes: Mutex::new(HashMap::new()),
                credit_cv: Condvar::new(),
                dead: AtomicBool::new(false),
                last_inbound: Mutex::new(Instant::now()),
                next_nonce: AtomicU64::new(1),
                next_local_id: AtomicU64::new(1),
                config,
                on_activity,
                close_hook,
            }),
        }
    }

    /// Declare the tunnel dead from outside (shutdown): the
    /// fail-all pass runs, in-flight streams fail with their
    /// trip ids preserved.
    pub fn close(&self) {
        self.shared.kill();
    }

    pub fn is_dead(&self) -> bool {
        self.shared.is_dead()
    }

    /// Originate a stream: allocate the next client id, register
    /// the row, and send the `open` frame. Fails fast with
    /// [`StreamError::TunnelLost`] (trip preserved) when the
    /// tunnel is already dead — a caller never waits on a corpse.
    pub fn open_stream(
        &self,
        port: u16,
        intent: &OpenIntent,
        trip: Option<String>,
    ) -> Result<MuxStream, StreamError> {
        if self.shared.is_dead() {
            return Err(StreamError::TunnelLost { trip });
        }
        // Client-chosen ids are odd (record §4): 1, 3, 5, …
        let n = self.shared.next_local_id.fetch_add(2, Ordering::SeqCst);
        let id = StreamId(n);
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let mut table = self.shared.table.lock().unwrap();
            table
                .open_local(id, port, trip.clone())
                .map_err(|e| StreamError::Protocol(e.to_string()))?;
        }
        self.shared.inboxes.lock().unwrap().insert(id, tx);
        let frame = match intent {
            OpenIntent::Request { method, path } => {
                StreamFrame::open_request(id, port, method.clone(), path.clone(), trip.clone())
            }
            OpenIntent::RawConnect => StreamFrame::open_connect(id, port, trip.clone()),
        };
        if let Err(e) = self.shared.send_frame(&frame) {
            self.shared.table.lock().unwrap().reap(id);
            self.shared.inboxes.lock().unwrap().remove(&id);
            return Err(e);
        }
        Ok(MuxStream::new(id, trip, self.shared.clone(), rx))
    }

    /// Run the session: a reader loop on the calling thread and a
    /// keepalive pinger beside it. Returns when the tunnel dies
    /// (EOF, transport error, or liveness expiry) — by the time
    /// it returns, every in-flight stream has been failed with
    /// its trip id preserved.
    pub fn run<R: Read + Send>(self, reader: R, handler: StreamHandler) {
        let pinger = {
            let shared = self.shared.clone();
            std::thread::spawn(move || pinger_loop(&shared))
        };
        self.reader_loop(reader, &handler);
        self.shared.kill();
        let _ = pinger.join();
    }

    fn reader_loop<R: Read>(&self, mut reader: R, handler: &StreamHandler) {
        loop {
            if self.shared.is_dead() {
                return;
            }
            let frame = match read_frame(&mut reader) {
                Ok(f) => f,
                Err(_) => return, // EOF or transport error: dead tunnel
            };
            *self.shared.last_inbound.lock().unwrap() = Instant::now();
            if let Some(activity) = &self.shared.on_activity {
                activity();
            }
            if !fleet_proxy_proto::mux::is_stream_frame(&frame) {
                // A mux session carries stream frames only. A
                // pooled-path frame here means the peer is not
                // speaking the family it negotiated — the session
                // ends (its streams fail via the kill in `run`).
                return;
            }
            let parsed = match fleet_proxy_proto::mux::parse_stream_frame(&frame) {
                Ok(p) => p,
                Err(_) => {
                    // Malformed stream frame: if it names a
                    // stream, reset that stream; tunnel-scoped
                    // junk is dropped. The tunnel itself survives
                    // one bad frame.
                    if let Some(n) = frame
                        .get("stream")
                        .and_then(|v| v.as_u64())
                        .filter(|n| *n != 0)
                    {
                        let id = StreamId(n);
                        self.deliver_reset(id, RST_PROTOCOL_ERROR);
                        let _ = self.shared.send_frame(&StreamFrame::Rst {
                            stream: id,
                            reason: RST_PROTOCOL_ERROR.to_string(),
                        });
                    }
                    continue;
                }
            };
            match parsed {
                StreamFrame::Open {
                    stream,
                    port,
                    method,
                    path,
                    trip,
                } => {
                    // parse_stream_frame already validated the
                    // intent; the half-head arm is unreachable in
                    // practice and stays as a guard.
                    let intent = match (method, path) {
                        (Some(m), Some(p)) => OpenIntent::Request { method: m, path: p },
                        (None, None) => OpenIntent::RawConnect,
                        _ => {
                            let _ = self.shared.send_frame(&StreamFrame::Rst {
                                stream,
                                reason: RST_PROTOCOL_ERROR.to_string(),
                            });
                            continue;
                        }
                    };
                    let opened = {
                        let mut table = self.shared.table.lock().unwrap();
                        table.open(stream, port, trip.clone())
                    };
                    if opened.is_err() {
                        let _ = self.shared.send_frame(&StreamFrame::Rst {
                            stream,
                            reason: RST_PROTOCOL_ERROR.to_string(),
                        });
                        continue;
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.shared.inboxes.lock().unwrap().insert(stream, tx);
                    let handle = MuxStream::new(stream, trip, self.shared.clone(), rx);
                    let info = OpenInfo {
                        stream,
                        port,
                        intent,
                        trip: handle.trip().map(|s| s.to_string()),
                    };
                    let handler = handler.clone();
                    std::thread::spawn(move || handler(handle, info));
                }
                StreamFrame::Data {
                    stream,
                    fin,
                    status,
                    ..
                } => {
                    let payload = match parsed.payload() {
                        Ok(Some(p)) => p,
                        _ => continue, // parse_stream_frame already validated it
                    };
                    let len = payload.len();
                    let overrun = {
                        let mut table = self.shared.table.lock().unwrap();
                        table.note_data_in(stream, len).is_err()
                    };
                    if overrun {
                        // The peer overran the window we granted
                        // (or the stream is unknown): protocol
                        // violation — reset the stream, keep the
                        // tunnel.
                        self.deliver_reset(stream, RST_PROTOCOL_ERROR);
                        let _ = self.shared.send_frame(&StreamFrame::Rst {
                            stream,
                            reason: RST_PROTOCOL_ERROR.to_string(),
                        });
                        continue;
                    }
                    self.deliver(
                        stream,
                        StreamEvent::Data {
                            bytes: payload,
                            status,
                        },
                    );
                    if fin {
                        self.shared.table.lock().unwrap().note_fin_in(stream);
                        self.deliver(stream, StreamEvent::Fin);
                    }
                }
                StreamFrame::Window { stream, credit } => {
                    let mut table = self.shared.table.lock().unwrap();
                    let _ = table.note_window(stream, credit);
                    drop(table);
                    self.shared.credit_cv.notify_all();
                }
                StreamFrame::Rst { stream, reason } => {
                    self.deliver_reset(stream, &reason);
                    self.shared.credit_cv.notify_all();
                }
                StreamFrame::Ping { nonce } => {
                    let _ = self.shared.send_frame(&StreamFrame::Pong { nonce });
                }
                StreamFrame::Pong { .. } => {
                    // Liveness was already stamped on receipt.
                }
            }
        }
    }

    fn deliver(&self, id: StreamId, event: StreamEvent) {
        let inboxes = self.shared.inboxes.lock().unwrap();
        if let Some(tx) = inboxes.get(&id) {
            let _ = tx.send(event);
        }
    }

    /// Deliver a reset to the stream's worker and record it in
    /// the table (row out, tombstone in) so blocked senders wake
    /// with the reason.
    fn deliver_reset(&self, id: StreamId, reason: &str) {
        {
            let mut table = self.shared.table.lock().unwrap();
            table.note_reset(id, reason);
        }
        self.deliver(id, StreamEvent::Rst(reason.to_string()));
    }
}

/// The keepalive pinger: sends `ping` on the configured cadence
/// and declares the tunnel dead when no inbound frame has arrived
/// within the liveness horizon (a silently half-open peer — the
/// transport itself has produced no error to notice).
fn pinger_loop(shared: &Arc<Shared>) {
    let tick = (shared.config.keepalive / 4)
        .max(Duration::from_millis(5))
        .min(Duration::from_millis(250));
    // Backdated so the first ping goes out on the first tick.
    let mut last_ping = Instant::now()
        .checked_sub(shared.config.keepalive)
        .unwrap_or_else(Instant::now);
    loop {
        if shared.is_dead() {
            return;
        }
        std::thread::sleep(tick);
        if shared.is_dead() {
            return;
        }
        let now = Instant::now();
        let last_inbound = *shared.last_inbound.lock().unwrap();
        if now.duration_since(last_inbound) > shared.config.liveness_timeout {
            shared.kill();
            return;
        }
        if now.duration_since(last_ping) >= shared.config.keepalive {
            last_ping = now;
            let nonce = shared.next_nonce.fetch_add(1, Ordering::SeqCst);
            if shared.send_frame(&StreamFrame::Ping { nonce }).is_err() {
                return; // send_frame already ran the fail-all pass
            }
        }
    }
}

/// One live stream, owned by its worker. All methods take `&self`
/// — the handle is shared (`Arc<MuxStream>`) when a raw stream's
/// two pump directions need it at once.
///
/// Dropping a handle whose stream is still open resets the
/// stream as cancelled: an abandoned stream never lingers in the
/// peer's table.
pub struct MuxStream {
    id: StreamId,
    trip: Option<String>,
    shared: Arc<Shared>,
    inbox: Mutex<std::sync::mpsc::Receiver<StreamEvent>>,
    /// Terminal receive state, latched: once a stream has ended
    /// (fin, reset, tunnel loss), reads keep reporting that end.
    recv_end: Mutex<Option<Result<(), StreamError>>>,
    /// Response status carried by the peer's first data frame.
    status: Mutex<Option<u16>>,
    /// Response status to stamp on this side's first data frame.
    status_out: Mutex<Option<u16>>,
    fin_out_sent: AtomicBool,
}

impl std::fmt::Debug for MuxStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MuxStream")
            .field("id", &self.id)
            .field("trip", &self.trip)
            .finish_non_exhaustive()
    }
}

impl MuxStream {
    fn new(
        id: StreamId,
        trip: Option<String>,
        shared: Arc<Shared>,
        inbox: std::sync::mpsc::Receiver<StreamEvent>,
    ) -> MuxStream {
        MuxStream {
            id,
            trip,
            shared,
            inbox: Mutex::new(inbox),
            recv_end: Mutex::new(None),
            status: Mutex::new(None),
            status_out: Mutex::new(None),
            fin_out_sent: AtomicBool::new(false),
        }
    }

    pub fn id(&self) -> StreamId {
        self.id
    }

    pub fn trip(&self) -> Option<&str> {
        self.trip.as_deref()
    }

    /// The response status the peer stamped on its first data
    /// frame, once one has been received.
    pub fn status(&self) -> Option<u16> {
        *self.status.lock().unwrap()
    }

    /// Stamp `status` on this side's first outbound data frame —
    /// the terminating side's response head (the mux analogue of
    /// the pooled `Response`'s status). Call before the first
    /// send; a later call has no frame left to ride and is
    /// ignored.
    pub fn set_status(&self, status: u16) {
        let mut slot = self.status_out.lock().unwrap();
        if slot.is_none() {
            *slot = Some(status);
        }
    }

    /// Receive the next payload chunk. `Ok(None)` is the peer's
    /// `fin`: end of data in its direction. `timeout` bounds the
    /// wait; expiry is [`StreamError::Timeout`] and the caller
    /// decides whether that is terminal (the record makes the
    /// ceiling a reset: see [`MuxStream::read_to_fin`]).
    ///
    /// Receiving does NOT return credit: the worker grants it
    /// back with [`MuxStream::consumed`] once it has actually
    /// consumed the bytes — that gap is the backpressure bound.
    pub fn recv_chunk(&self, timeout: Duration) -> Result<Option<Vec<u8>>, StreamError> {
        if let Some(end) = self.recv_end.lock().unwrap().clone() {
            return match end {
                Ok(()) => Ok(None),
                Err(e) => Err(e),
            };
        }
        let event = {
            let inbox = self.inbox.lock().unwrap();
            inbox.recv_timeout(timeout)
        };
        match event {
            Ok(StreamEvent::Data { bytes, status }) => {
                if let Some(status) = status {
                    let mut slot = self.status.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(status);
                    }
                }
                Ok(Some(bytes))
            }
            Ok(StreamEvent::Fin) => {
                *self.recv_end.lock().unwrap() = Some(Ok(()));
                Ok(None)
            }
            Ok(StreamEvent::Rst(reason)) => {
                let err = StreamError::Reset { reason };
                *self.recv_end.lock().unwrap() = Some(Err(err.clone()));
                Err(err)
            }
            Ok(StreamEvent::TunnelLost(trip)) => {
                let err = StreamError::TunnelLost { trip };
                *self.recv_end.lock().unwrap() = Some(Err(err.clone()));
                Err(err)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(StreamError::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // The session ended without a per-stream event
                // (the table was already empty for this stream —
                // e.g. it completed both fins and the tunnel then
                // died): report the tunnel loss with our own trip.
                let err = StreamError::TunnelLost {
                    trip: self.trip.clone(),
                };
                *self.recv_end.lock().unwrap() = Some(Err(err.clone()));
                Err(err)
            }
        }
    }

    /// Grant back credit for `len` bytes this worker has consumed
    /// of what it received: the `window` frame that reopens the
    /// peer's sending. Consuming more than was received grants
    /// only what the table still holds ungranted.
    pub fn consumed(&self, len: u64) {
        let credit = self
            .shared
            .table
            .lock()
            .unwrap()
            .note_consumed(self.id, len);
        if credit > 0 {
            let _ = self.shared.send_frame(&StreamFrame::Window {
                stream: self.id,
                credit,
            });
        }
    }

    /// Read this stream's inbound direction to its `fin`,
    /// granting credit back as bytes are consumed. `max_bytes`
    /// bounds the assembly (a peer streaming past it is reset as
    /// a protocol violation); the wait for each chunk is bounded
    /// by the session ceiling, whose expiry resets the stream
    /// with the record's `timeout` reason.
    pub fn read_to_fin(&self, max_bytes: usize) -> Result<Vec<u8>, StreamError> {
        let ceiling = self.shared.config.ceiling.timeout();
        let mut out = Vec::new();
        loop {
            match self.recv_chunk(ceiling) {
                Ok(Some(bytes)) => {
                    let len = bytes.len() as u64;
                    out.extend_from_slice(&bytes);
                    self.consumed(len);
                    if out.len() > max_bytes {
                        self.reset(RST_PROTOCOL_ERROR);
                        return Err(StreamError::Protocol(format!(
                            "stream body exceeds the {max_bytes}-byte assembly bound"
                        )));
                    }
                }
                Ok(None) => return Ok(out),
                Err(StreamError::Timeout) => {
                    self.reset(RST_TIMEOUT);
                    return Err(StreamError::Timeout);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Send one chunk, splitting at the proto frame bound and
    /// waiting for credit whenever the granted window runs out —
    /// the wait is bounded by the session ceiling, and a reset or
    /// a dead tunnel interrupts it with that error. This is the
    /// only send path, so no caller can overrun the peer's
    /// window by construction.
    pub fn send_chunk(&self, bytes: &[u8]) -> Result<(), StreamError> {
        if bytes.is_empty() {
            return Ok(());
        }
        for piece in bytes.chunks(fleet_proxy_proto::mux::MAX_DATA_PAYLOAD_BYTES) {
            self.wait_credit(piece.len())?;
            {
                let mut table = self.shared.table.lock().unwrap();
                table
                    .note_data_out(self.id, piece.len())
                    .map_err(|e| StreamError::Protocol(e.to_string()))?;
            }
            let status = self.status_out.lock().unwrap().take();
            let mut frame = StreamFrame::data(self.id, piece, false)
                .map_err(|e| StreamError::Protocol(e.to_string()))?;
            if let StreamFrame::Data { status: slot, .. } = &mut frame {
                *slot = status;
            }
            self.shared
                .send_frame(&frame)
                .map_err(|_| StreamError::TunnelLost {
                    trip: self.trip.clone(),
                })?;
        }
        Ok(())
    }

    /// End this side's sending direction: a final empty data
    /// frame with `fin` (carrying the response status if none has
    /// been sent yet). Idempotent.
    pub fn close_send(&self) -> Result<(), StreamError> {
        if self.fin_out_sent.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let status = self.status_out.lock().unwrap().take();
        let mut frame = StreamFrame::data(self.id, &[], true)
            .map_err(|e| StreamError::Protocol(e.to_string()))?;
        if let StreamFrame::Data { status: slot, .. } = &mut frame {
            *slot = status;
        }
        self.shared
            .send_frame(&frame)
            .map_err(|_| StreamError::TunnelLost {
                trip: self.trip.clone(),
            })?;
        self.shared.table.lock().unwrap().note_fin_out(self.id);
        Ok(())
    }

    /// One-shot response for a request stream: stamp the status,
    /// send the whole body, and close the sending direction.
    pub fn respond(&self, status: u16, body: &[u8]) -> Result<(), StreamError> {
        self.set_status(status);
        self.send_chunk(body)?;
        self.close_send()
    }

    /// Reset the stream (cancellation, timeout, fast failure):
    /// the peer's worker and any blocked sender on this side wake
    /// with `reason`. Idempotent; a reset stream stays reset.
    pub fn reset(&self, reason: &str) {
        {
            let mut table = self.shared.table.lock().unwrap();
            if table.terminal_reason(self.id).is_some()
                || !table.contains(self.id)
                || table.is_complete(self.id)
            {
                return;
            }
            table.note_reset(self.id, reason);
        }
        let mut end = self.recv_end.lock().unwrap();
        if end.is_none() {
            *end = Some(Err(StreamError::Reset {
                reason: reason.to_string(),
            }));
        }
        drop(end);
        self.shared.credit_cv.notify_all();
        let _ = self.shared.send_frame(&StreamFrame::Rst {
            stream: self.id,
            reason: reason.to_string(),
        });
    }

    /// Wait until the peer's granted credit covers `want` bytes.
    fn wait_credit(&self, want: usize) -> Result<(), StreamError> {
        let deadline = Instant::now() + self.shared.config.ceiling.timeout();
        let mut table = self.shared.table.lock().unwrap();
        loop {
            if self.shared.is_dead() {
                return Err(StreamError::TunnelLost {
                    trip: self.trip.clone(),
                });
            }
            if let Some(reason) = table.terminal_reason(self.id) {
                return Err(StreamError::Reset {
                    reason: reason.to_string(),
                });
            }
            if !table.contains(self.id) {
                return Err(StreamError::Protocol(format!(
                    "stream {} is closed",
                    self.id.0
                )));
            }
            if table.send_credit(self.id) >= want as u64 {
                return Ok(());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(StreamError::Timeout);
            }
            let (guard, _) = self.shared.credit_cv.wait_timeout(table, left).unwrap();
            table = guard;
        }
    }
}

impl Drop for MuxStream {
    fn drop(&mut self) {
        // An abandoned stream is a cancelled stream: tell the
        // peer, then reap the row and the inbox so a late frame
        // for this id meets a clean table. A completed stream
        // (both fins) is reaped silently — an rst after fin
        // would contradict the exchange the peer just finished.
        let live = {
            let table = self.shared.table.lock().unwrap();
            table.contains(self.id) && !table.is_complete(self.id)
        };
        if live && !self.shared.is_dead() {
            let _ = self.shared.send_frame(&StreamFrame::Rst {
                stream: self.id,
                reason: RST_CANCELLED.to_string(),
            });
        }
        self.shared.table.lock().unwrap().reap(self.id);
        self.shared.inboxes.lock().unwrap().remove(&self.id);
        self.shared.credit_cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> StreamId {
        StreamId(n)
    }

    #[test]
    fn table_open_enforces_client_id_rules() {
        // Oracle: record §4 — stream ids are client-chosen, odd,
        // scoped to the tunnel; 0 is never a stream.
        let mut table = StreamTable::new();
        assert!(table.open(id(1), 4799, None).is_ok());
        assert!(table.open(id(2), 4799, None).is_err(), "even id refused");
        assert!(table.open(id(0), 4799, None).is_err(), "zero id refused");
        assert!(table.open(id(1), 4799, None).is_err(), "duplicate refused");
        assert!(table.open(id(3), 4801, Some("t-3".to_string())).is_ok());
        assert_eq!(table.len(), 2);
        assert_eq!(table.port_of(id(3)), Some(4801));
    }

    #[test]
    fn table_window_bounds_both_directions() {
        // Oracle: the proto crate's StreamWindow semantics —
        // 256 KiB initial credit, sends never overrun grants,
        // receives never overrun the window we granted, consumed
        // bytes become grantable credit.
        use fleet_proxy_proto::mux::{INITIAL_STREAM_WINDOW_BYTES, MAX_DATA_PAYLOAD_BYTES};
        let mut table = StreamTable::new();
        table.open(id(1), 4799, None).unwrap();
        assert_eq!(table.send_credit(id(1)), INITIAL_STREAM_WINDOW_BYTES);
        let chunks = INITIAL_STREAM_WINDOW_BYTES as usize / MAX_DATA_PAYLOAD_BYTES;
        for _ in 0..chunks {
            table.note_data_out(id(1), MAX_DATA_PAYLOAD_BYTES).unwrap();
            table.note_data_in(id(1), MAX_DATA_PAYLOAD_BYTES).unwrap();
        }
        assert_eq!(table.send_credit(id(1)), 0);
        assert!(
            table.note_data_out(id(1), 1).is_err(),
            "send overrun refused"
        );
        assert!(
            table.note_data_in(id(1), 1).is_err(),
            "receive overrun refused"
        );
        table.note_window(id(1), 100).unwrap();
        table.note_data_out(id(1), 100).unwrap();
        assert_eq!(table.note_consumed(id(1), 512), 512);
        table.note_data_in(id(1), 512).unwrap();
        // Unknown streams have no credit and refuse accounting.
        assert_eq!(table.send_credit(id(9)), 0);
        assert!(table.note_data_out(id(9), 1).is_err());
        assert!(table.note_data_in(id(9), 1).is_err());
    }

    #[test]
    fn table_fail_all_preserves_every_trip_id() {
        // Oracle: record §4 — a dead tunnel fails every in-flight
        // stream immediately, each with its own trip id preserved.
        let mut table = StreamTable::new();
        table.open(id(1), 4799, Some("trip-a".to_string())).unwrap();
        table.open(id(3), 4801, Some("trip-b".to_string())).unwrap();
        table.open(id(5), 4802, None).unwrap();
        let mut failed = table.fail_all();
        failed.sort();
        assert_eq!(
            failed,
            vec![
                (id(1), Some("trip-a".to_string())),
                (id(3), Some("trip-b".to_string())),
                (id(5), None),
            ]
        );
        assert!(table.is_empty());
        // A second pass fails nothing — the single-shot property.
        assert!(table.fail_all().is_empty());
    }

    #[test]
    fn table_reset_tombstones_the_reason_and_completion_needs_both_fins() {
        let mut table = StreamTable::new();
        table.open(id(1), 4799, None).unwrap();
        table.note_fin_in(id(1));
        assert!(!table.is_complete(id(1)));
        table.note_fin_out(id(1));
        assert!(table.is_complete(id(1)));

        table.open(id(3), 4799, None).unwrap();
        table.note_reset(id(3), RST_TIMEOUT);
        assert!(!table.contains(id(3)));
        assert_eq!(table.terminal_reason(id(3)), Some(RST_TIMEOUT));
        // Reopening a tombstoned id is refused until reaped.
        assert!(table.open(id(3), 4799, None).is_err());
        table.reap(id(3));
        assert!(table.open(id(3), 4799, None).is_ok());
    }

    #[test]
    fn default_config_carries_the_record_constants() {
        // Oracle: record §4 — KEEPALIVE_S = 180, ceiling default
        // 10 s (proto StreamCeiling::default).
        let config = MuxConfig::default();
        assert_eq!(config.keepalive, Duration::from_secs(MUX_KEEPALIVE_S));
        assert_eq!(config.liveness_timeout, config.keepalive * 2);
        assert_eq!(config.ceiling, StreamCeiling::default());
    }
}
