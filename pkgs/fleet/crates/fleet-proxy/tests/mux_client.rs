//! Integration tests for the client-side mux call path
//! (issue #5756, seam record §7 step 3): the client's
//! `mux_roundtrip` against a broker endpoint running the
//! crate's own terminating machinery with a test handler.
//!
//! Oracles are the record's step-3 terms: a request rides one
//! stream end to end (status on the first frame, body to fin,
//! trip id preserved); a stream that fails before any
//! response data is retried exactly once on a fresh stream; a
//! failure after partial response data is terminal — no
//! retry can double-deliver an answer the caller half-saw.

#![cfg(unix)]

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use fleet_proxy::client::mux_roundtrip;
use fleet_proxy::mux::{MuxConfig, MuxSession, MuxStream, OpenInfo, StreamHandler};
use fleet_proxy_proto::mux::{OpenIntent, RST_CANCELLED, StreamCeiling};
use fleet_proxy_proto::{MAX_FRAME_BYTES, Request};

fn fast_config() -> MuxConfig {
    MuxConfig {
        keepalive: Duration::from_millis(50),
        liveness_timeout: Duration::from_millis(600),
        ceiling: StreamCeiling::new(Duration::from_secs(5)).unwrap(),
    }
}

fn shutdown_hook(stream: &UnixStream) -> Arc<dyn Fn() + Send + Sync> {
    let stream = stream.try_clone().unwrap();
    Arc::new(move || {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    })
}

/// A broker endpoint on one end of a socket pair; the other
/// end is handed to `mux_roundtrip`. Returns the client-side
/// socket, a kill clone for teardown, and the broker thread.
fn broker(handler: StreamHandler) -> (UnixStream, UnixStream, JoinHandle<()>) {
    let (client_side, broker_side) = UnixStream::pair().unwrap();
    let kill = client_side.try_clone().unwrap();
    let session = MuxSession::new(
        Box::new(broker_side.try_clone().unwrap()),
        fast_config(),
        None,
        Some(shutdown_hook(&broker_side)),
    );
    let thread = std::thread::spawn(move || session.run(broker_side, handler));
    (client_side, kill, thread)
}

fn teardown(kill: UnixStream, thread: JoinHandle<()>) {
    let _ = kill.shutdown(std::net::Shutdown::Both);
    let _ = thread.join();
}

fn request(body: Option<serde_json::Value>) -> Request {
    Request {
        method: "POST".to_string(),
        path: "/queue/claim".to_string(),
        body,
        port: Some(4801),
        token_file: None,
        trip: Some("trip-step3".to_string()),
        transport: None,
        cf_ok: None,
        ctx: None,
    }
}

/// What the echo handler records per stream: (method, path,
/// trip).
type Seen = Arc<Mutex<Vec<(String, String, Option<String>)>>>;

/// The terminating handler shape serve uses: assemble the
/// request body to fin, then answer in one response. Records
/// what it saw for the test to assert on.
fn echo_handler(seen: Seen) -> StreamHandler {
    Arc::new(move |stream: MuxStream, info: OpenInfo| {
        let (method, path) = match &info.intent {
            OpenIntent::Request { method, path } => (method.clone(), path.clone()),
            OpenIntent::RawConnect => (String::new(), String::new()),
        };
        let body = stream.read_to_fin(MAX_FRAME_BYTES).unwrap();
        seen.lock().unwrap().push((method, path, info.trip.clone()));
        let echoed = serde_json::json!({
            "received": String::from_utf8_lossy(&body),
        });
        stream
            .respond(200, &serde_json::to_vec(&echoed).unwrap())
            .unwrap();
    })
}

#[test]
fn mux_roundtrip_delivers_request_and_response() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (stream, kill, thread) = broker(echo_handler(seen.clone()));
    let resp = mux_roundtrip(stream, &request(Some(serde_json::json!({"task": 7}))))
        .expect("mux roundtrip");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, serde_json::json!({ "received": "{\"task\":7}" }));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "POST");
    assert_eq!(seen[0].1, "/queue/claim");
    assert_eq!(seen[0].2.as_deref(), Some("trip-step3"));
    teardown(kill, thread);
}

#[test]
fn mux_roundtrip_retries_once_on_a_fresh_stream() {
    let opens = Arc::new(AtomicUsize::new(0));
    let counter = opens.clone();
    let handler: StreamHandler = Arc::new(move |stream: MuxStream, _info: OpenInfo| {
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            // First stream dies before any response data.
            stream.reset(RST_CANCELLED);
            return;
        }
        let _ = stream.read_to_fin(MAX_FRAME_BYTES).unwrap();
        stream.respond(200, br#"{"ok":true}"#).unwrap();
    });
    let (stream, kill, thread) = broker(handler);
    let resp = mux_roundtrip(stream, &request(None)).expect("retry succeeds");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, serde_json::json!({ "ok": true }));
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    teardown(kill, thread);
}

#[test]
fn mux_roundtrip_does_not_retry_after_partial_response() {
    let opens = Arc::new(AtomicUsize::new(0));
    let counter = opens.clone();
    let handler: StreamHandler = Arc::new(move |stream: MuxStream, _info: OpenInfo| {
        counter.fetch_add(1, Ordering::SeqCst);
        let _ = stream.read_to_fin(MAX_FRAME_BYTES).unwrap();
        // Half an answer, then the stream dies: the caller
        // saw response data, so a retry is not safe.
        stream.send_chunk(b"{\"partial\":").unwrap();
        stream.reset(RST_CANCELLED);
    });
    let (stream, kill, thread) = broker(handler);
    let err = mux_roundtrip(stream, &request(None)).expect_err("partial response is terminal");
    assert!(
        format!("{err}").contains("cancelled"),
        "unexpected error: {err}"
    );
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    teardown(kill, thread);
}

#[test]
fn mux_roundtrip_empty_response_body_is_null() {
    let handler: StreamHandler = Arc::new(move |stream: MuxStream, _info: OpenInfo| {
        let _ = stream.read_to_fin(MAX_FRAME_BYTES).unwrap();
        stream.respond(204, &[]).unwrap();
    });
    let (stream, kill, thread) = broker(handler);
    let resp = mux_roundtrip(stream, &request(None)).expect("mux roundtrip");
    assert_eq!(resp.status, 204);
    assert_eq!(resp.body, serde_json::Value::Null);
    teardown(kill, thread);
}

/// #15552 (record §7 step 4): the pooled frame's per-request
/// routing fields ride the open as `meta`, and the terminating
/// side's response head — transport and the trip echo — comes
/// back onto the caller's `Response`.
#[test]
fn mux_roundtrip_carries_request_meta_and_reads_the_echoed_head_15552() {
    use fleet_proxy::mux::ResponseHead;
    use fleet_proxy_proto::mux::RequestMeta;
    let seen: Arc<Mutex<Vec<RequestMeta>>> = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    let handler: StreamHandler = Arc::new(move |stream: MuxStream, info: OpenInfo| {
        let _ = stream.read_to_fin(MAX_FRAME_BYTES).unwrap();
        record.lock().unwrap().push(info.meta.clone());
        stream
            .respond_with(
                ResponseHead {
                    status: Some(201),
                    transport: Some("ts".to_string()),
                    trip: info.trip.clone(),
                },
                br#"{"ok":true}"#,
            )
            .unwrap();
    });
    let (stream, kill, thread) = broker(handler);
    let mut ctx = std::collections::BTreeMap::new();
    ctx.insert("FLEET_CTX_ID".to_string(), "ctx-15552".to_string());
    let mut req = request(None);
    req.token_file = Some("/run/fleet/secrets.token".to_string());
    req.transport = Some("ts".to_string());
    req.cf_ok = Some(false);
    req.ctx = Some(ctx.clone());
    let resp = mux_roundtrip(stream, &req).expect("mux roundtrip");
    assert_eq!(resp.status, 201);
    assert_eq!(resp.transport.as_deref(), Some("ts"));
    assert_eq!(resp.body, serde_json::json!({"ok": true}));
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.as_slice(),
        &[RequestMeta {
            token_file: Some("/run/fleet/secrets.token".to_string()),
            transport: Some("ts".to_string()),
            cf_ok: Some(false),
            ctx: Some(ctx),
        }]
    );
    teardown(kill, thread);
}

/// #15552: a reply whose echoed trip is not this call's is never
/// handed to the caller.
#[test]
fn mux_roundtrip_refuses_a_reply_echoing_another_trip_15552() {
    use fleet_proxy::mux::ResponseHead;
    let handler: StreamHandler = Arc::new(move |stream: MuxStream, _info: OpenInfo| {
        let _ = stream.read_to_fin(MAX_FRAME_BYTES).unwrap();
        stream
            .respond_with(
                ResponseHead {
                    status: Some(200),
                    transport: None,
                    trip: Some("someone-elses-trip".to_string()),
                },
                br#"{"ok":true}"#,
            )
            .unwrap();
    });
    let (stream, kill, thread) = broker(handler);
    let err = mux_roundtrip(stream, &request(None)).expect_err("mismatched trip echo");
    let text = err.to_string();
    assert!(text.contains("someone-elses-trip"), "{text}");
    assert!(text.contains("trip-step3"), "{text}");
    teardown(kill, thread);
}
