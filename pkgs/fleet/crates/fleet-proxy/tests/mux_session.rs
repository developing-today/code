//! Integration tests for the proxy-side mux (issue #5756, seam
//! record §7 step 2): two `MuxSession` endpoints back to back
//! over a Unix socket pair — one originates streams, the other
//! terminates them with a test handler.
//!
//! Oracles are the seam record's §4 rules and §6 acceptance
//! shapes, at session scale: interleaving never mixes streams,
//! per-stream credit bounds backpressure, cancellation and the
//! per-stream ceiling are `rst`, and a dead tunnel fails every
//! in-flight stream fast (≤2 s, PR #6490's bound) with each
//! stream's trip id preserved.

#![cfg(unix)]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use fleet_proxy::mux::{MuxConfig, MuxSession, MuxStream, OpenInfo, StreamError, StreamHandler};
use fleet_proxy_proto::mux::{
    OpenIntent, StreamCeiling, StreamId, RST_CANCELLED, RST_TIMEOUT,
};

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

/// A linked pair: `a` originates, `b` terminates with `handler`.
/// Returns both sessions plus a kill switch for the raw socket.
fn link(
    a_config: MuxConfig,
    b_config: MuxConfig,
    handler: StreamHandler,
) -> (MuxSession, MuxSession, UnixStream) {
    let (sa, sb) = UnixStream::pair().unwrap();
    let kill = sa.try_clone().unwrap();
    let a = MuxSession::new(
        Box::new(sa.try_clone().unwrap()),
        a_config,
        None,
        Some(shutdown_hook(&sa)),
    );
    let b = MuxSession::new(
        Box::new(sb.try_clone().unwrap()),
        b_config,
        None,
        Some(shutdown_hook(&sb)),
    );
    {
        let b_run = b.clone();
        std::thread::spawn(move || b_run.run(sb, handler));
    }
    {
        let a_run = a.clone();
        let noop: StreamHandler = Arc::new(|_stream, _info| {});
        std::thread::spawn(move || a_run.run(sa, noop));
    }
    (a, b, kill)
}

/// Read a stream to its fin, granting credit as bytes land;
/// returns the assembled body.
fn read_all(stream: &MuxStream) -> Result<Vec<u8>, StreamError> {
    let mut out = Vec::new();
    loop {
        match stream.recv_chunk(Duration::from_secs(5)) {
            Ok(Some(bytes)) => {
                let len = bytes.len() as u64;
                out.extend_from_slice(&bytes);
                stream.consumed(len);
            }
            Ok(None) => return Ok(out),
            Err(e) => return Err(e),
        }
    }
}

/// Echo handler: read the request body to fin, respond 200 with
/// the same bytes.
fn echo_handler() -> StreamHandler {
    Arc::new(|stream: MuxStream, _info: OpenInfo| {
        if let Ok(body) = stream.read_to_fin(4 * 1024 * 1024) {
            let _ = stream.respond(200, &body);
        }
    })
}

fn pattern(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| seed ^ (i % 251) as u8).collect()
}

#[test]
fn interleaved_streams_stay_isolated() {
    // Oracle: record §4 — frames interleave by stream id; no
    // stream's bytes ever land on another stream.
    let (a, _b, _kill) = link(fast_config(), fast_config(), echo_handler());
    // Let keepalive pings flow both ways first: a healthy idle
    // session must still be alive well past one interval.
    std::thread::sleep(Duration::from_millis(200));
    assert!(!a.is_dead(), "idle session died despite ping/pong");

    let s1 = a
        .open_stream(
            4799,
            &OpenIntent::Request {
                method: "POST".to_string(),
                path: "/one".to_string(),
            },
            Some("trip-one".to_string()),
        )
        .unwrap();
    let s2 = a
        .open_stream(
            4799,
            &OpenIntent::Request {
                method: "POST".to_string(),
                path: "/two".to_string(),
            },
            Some("trip-two".to_string()),
        )
        .unwrap();
    assert_eq!(s1.id(), StreamId(1));
    assert_eq!(s2.id(), StreamId(3));

    let body1 = pattern(0x11, 100_000);
    let body2 = pattern(0x77, 100_000);
    // Interleave the two streams' chunks on purpose.
    s1.send_chunk(&body1[..40_000]).unwrap();
    s2.send_chunk(&body2[..40_000]).unwrap();
    s1.send_chunk(&body1[40_000..]).unwrap();
    s2.send_chunk(&body2[40_000..]).unwrap();
    s1.close_send().unwrap();
    s2.close_send().unwrap();

    assert_eq!(read_all(&s1).unwrap(), body1);
    assert_eq!(s1.status(), Some(200));
    assert_eq!(read_all(&s2).unwrap(), body2);
    assert_eq!(s2.status(), Some(200));
}

#[test]
fn backpressure_bounds_one_stream_and_not_the_tunnel() {
    // Oracle: record §4 — the 256 KiB receive window is the
    // backpressure bound: a sender blocks past it until the
    // consumer grants credit, and other streams flow meanwhile.
    let release = Arc::new(AtomicBool::new(false));
    let handler: StreamHandler = {
        let release = release.clone();
        Arc::new(move |stream: MuxStream, info: OpenInfo| {
            if info.port == 4801 {
                // Stall: read one chunk, then wait for the test
                // to release us before consuming anything (the
                // held chunk counts against the window — nothing
                // is consumed, so no credit flows back).
                let mut body = Vec::new();
                if let Ok(Some(first)) = stream.recv_chunk(Duration::from_secs(5)) {
                    body.extend_from_slice(&first);
                    while !release.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    stream.consumed(first.len() as u64);
                }
                if let Ok(rest) = stream.read_to_fin(4 * 1024 * 1024) {
                    body.extend_from_slice(&rest);
                    let _ = stream.respond(200, &body);
                }
            } else if let Ok(body) = stream.read_to_fin(4 * 1024 * 1024) {
                let _ = stream.respond(200, &body);
            }
        })
    };
    let (a, _b, _kill) = link(fast_config(), fast_config(), handler);

    let big = a
        .open_stream(4801, &OpenIntent::RawConnect, Some("trip-big".to_string()))
        .unwrap();
    let payload = pattern(0x42, 300_000); // > the 256 KiB window
    let send_done = Arc::new(AtomicBool::new(false));
    let sender = {
        let send_done = send_done.clone();
        std::thread::spawn(move || {
            let r = big.send_chunk(&payload);
            send_done.store(true, Ordering::SeqCst);
            (big, r)
        })
    };
    // The send cannot complete while the consumer stalls: the
    // window is exhausted ~256 KiB in.
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        !send_done.load(Ordering::SeqCst),
        "send overran the granted window instead of blocking"
    );
    // A second stream flows freely on the same tunnel meanwhile.
    let small = a
        .open_stream(
            4799,
            &OpenIntent::Request {
                method: "POST".to_string(),
                path: "/small".to_string(),
            },
            None,
        )
        .unwrap();
    small.send_chunk(b"ping").unwrap();
    small.close_send().unwrap();
    assert_eq!(read_all(&small).unwrap(), b"ping");

    // Release the consumer: the blocked send completes and the
    // full body arrives intact.
    release.store(true, Ordering::SeqCst);
    let (big, sent) = sender.join().unwrap();
    sent.unwrap();
    big.close_send().unwrap();
    assert_eq!(read_all(&big).unwrap(), payload);
}

#[test]
fn cancellation_resets_only_the_cancelled_stream() {
    // Oracle: record §4 — cancellation is `rst` with the
    // cancelled reason; sibling streams are unaffected.
    let (tx, rx) = mpsc::channel();
    let handler: StreamHandler = {
        let tx = Mutex::new(tx);
        Arc::new(move |stream: MuxStream, info: OpenInfo| {
            if info.port == 4802 {
                let result = stream.read_to_fin(1024 * 1024);
                let _ = tx.lock().unwrap().send(result.map(|_| ()));
            } else if let Ok(body) = stream.read_to_fin(1024 * 1024) {
                let _ = stream.respond(200, &body);
            }
        })
    };
    let (a, _b, _kill) = link(fast_config(), fast_config(), handler);

    let doomed = a
        .open_stream(4802, &OpenIntent::RawConnect, Some("trip-doomed".to_string()))
        .unwrap();
    doomed.send_chunk(b"partial").unwrap();
    doomed.reset(RST_CANCELLED);
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Err(StreamError::Reset { reason })) => assert_eq!(reason, RST_CANCELLED),
        other => panic!("handler should observe the cancellation, got {other:?}"),
    }

    let alive = a
        .open_stream(
            4799,
            &OpenIntent::Request {
                method: "POST".to_string(),
                path: "/alive".to_string(),
            },
            None,
        )
        .unwrap();
    alive.send_chunk(b"still here").unwrap();
    alive.close_send().unwrap();
    assert_eq!(read_all(&alive).unwrap(), b"still here");
}

#[test]
fn stream_ceiling_resets_a_stalled_stream_as_timeout() {
    // Oracle: record §4 — the per-stream timeout ceiling is a
    // client-side-style timer whose expiry is `rst` (timeout).
    let mut b_config = fast_config();
    b_config.ceiling = StreamCeiling::new(Duration::from_millis(200)).unwrap();
    let (tx, rx) = mpsc::channel();
    let handler: StreamHandler = {
        let tx = Mutex::new(tx);
        Arc::new(move |stream: MuxStream, _info: OpenInfo| {
            let result = stream.read_to_fin(1024);
            let _ = tx.lock().unwrap().send(result);
        })
    };
    let (a, _b, _kill) = link(fast_config(), b_config, handler);

    // Open and send nothing: the terminating side's ceiling
    // expires and it resets the stream with the timeout reason.
    let stalled = a
        .open_stream(4803, &OpenIntent::RawConnect, Some("trip-stall".to_string()))
        .unwrap();
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Err(StreamError::Timeout)) => {}
        other => panic!("handler should hit its ceiling, got {other:?}"),
    }
    match stalled.recv_chunk(Duration::from_secs(5)) {
        Err(StreamError::Reset { reason }) => assert_eq!(reason, RST_TIMEOUT),
        other => panic!("originator should see the timeout rst, got {other:?}"),
    }
}

#[test]
fn dead_tunnel_fails_every_stream_fast_with_its_trip_id() {
    // Oracle: record §4 + §6 item 3 — a dead tunnel fails every
    // in-flight stream immediately (≤2 s, PR #6490's bound),
    // each failure preserving the stream's own trip id.
    let (tx, rx) = mpsc::channel();
    let handler: StreamHandler = {
        let tx = Mutex::new(tx);
        Arc::new(move |stream: MuxStream, _info: OpenInfo| {
            let result = stream.read_to_fin(1024 * 1024);
            let _ = tx.lock().unwrap().send(result);
        })
    };
    let (a, _b, kill) = link(fast_config(), fast_config(), handler);

    let s1 = a
        .open_stream(4804, &OpenIntent::RawConnect, Some("trip-one".to_string()))
        .unwrap();
    let s2 = a
        .open_stream(4804, &OpenIntent::RawConnect, Some("trip-two".to_string()))
        .unwrap();
    s1.send_chunk(b"unfinished").unwrap();
    s2.send_chunk(b"also unfinished").unwrap();
    // Give the opens time to reach the terminating side.
    std::thread::sleep(Duration::from_millis(100));

    let t0 = Instant::now();
    kill.shutdown(std::net::Shutdown::Both).unwrap();

    // Originating side: both streams fail, trips preserved.
    for (stream, trip) in [(&s1, "trip-one"), (&s2, "trip-two")] {
        match stream.recv_chunk(Duration::from_secs(5)) {
            Err(StreamError::TunnelLost { trip: got }) => {
                assert_eq!(got.as_deref(), Some(trip))
            }
            other => panic!("stream should fail tunnel-lost, got {other:?}"),
        }
    }
    // Terminating side: its workers fail the same way.
    let mut trips = Vec::new();
    for _ in 0..2 {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Err(StreamError::TunnelLost { trip })) => trips.push(trip),
            other => panic!("handler should fail tunnel-lost, got {other:?}"),
        }
    }
    trips.sort();
    assert_eq!(
        trips,
        vec![Some("trip-one".to_string()), Some("trip-two".to_string())]
    );
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "fast-fail took {:?}, past the 2 s bound",
        t0.elapsed()
    );
    // And a fresh open on the corpse fails fast, trip intact.
    match a.open_stream(4804, &OpenIntent::RawConnect, Some("trip-late".to_string())) {
        Err(StreamError::TunnelLost { trip }) => {
            assert_eq!(trip.as_deref(), Some("trip-late"))
        }
        other => panic!("open on a dead tunnel should fail fast, got {other:?}"),
    }
}

#[test]
fn silent_peer_is_declared_dead_by_the_liveness_horizon() {
    // Oracle: record §4 — keepalive is application-level; a peer
    // that stops answering (socket open, nobody home) is a dead
    // tunnel once the horizon passes, not a hang.
    let (sa, sb) = UnixStream::pair().unwrap();
    // sb is never read or written: the peer is silently gone.
    let _sb_hold = sb;
    let a = MuxSession::new(
        Box::new(sa.try_clone().unwrap()),
        fast_config(),
        None,
        Some(shutdown_hook(&sa)),
    );
    {
        let a_run = a.clone();
        let noop: StreamHandler = Arc::new(|_stream, _info| {});
        std::thread::spawn(move || a_run.run(sa, noop));
    }
    let stream = a
        .open_stream(4805, &OpenIntent::RawConnect, Some("trip-silent".to_string()))
        .unwrap();
    let t0 = Instant::now();
    while !a.is_dead() {
        assert!(
            t0.elapsed() < Duration::from_secs(3),
            "liveness horizon never fired"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    match stream.recv_chunk(Duration::from_secs(2)) {
        Err(StreamError::TunnelLost { trip }) => {
            assert_eq!(trip.as_deref(), Some("trip-silent"))
        }
        other => panic!("stream should fail tunnel-lost, got {other:?}"),
    }
}

#[test]
fn concurrent_close_runs_the_fail_all_pass_once() {
    // Oracle: record §4 — reconnect/failure handling is
    // single-flight: however many threads notice the death, each
    // stream receives exactly one terminal failure.
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let session = MuxSession::new(
        Box::new(Sink(Arc::new(Mutex::new(Vec::new())))),
        fast_config(),
        None,
        None,
    );
    let s1 = session
        .open_stream(4806, &OpenIntent::RawConnect, Some("trip-x".to_string()))
        .unwrap();
    let s2 = session
        .open_stream(4806, &OpenIntent::RawConnect, Some("trip-y".to_string()))
        .unwrap();
    let closers: Vec<_> = (0..8)
        .map(|_| {
            let session = session.clone();
            std::thread::spawn(move || session.close())
        })
        .collect();
    for c in closers {
        c.join().unwrap();
    }
    assert!(session.is_dead());
    for (stream, trip) in [(&s1, "trip-x"), (&s2, "trip-y")] {
        match stream.recv_chunk(Duration::from_secs(2)) {
            Err(StreamError::TunnelLost { trip: got }) => {
                assert_eq!(got.as_deref(), Some(trip))
            }
            other => panic!("expected one tunnel-lost failure, got {other:?}"),
        }
        // The terminal state is latched: a second read reports
        // the same single failure, never a second event.
        match stream.recv_chunk(Duration::from_secs(2)) {
            Err(StreamError::TunnelLost { .. }) => {}
            other => panic!("terminal state should latch, got {other:?}"),
        }
    }
}
