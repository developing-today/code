//! Step-5 acceptance harness for the proxy-mux path — issue #5756 §6
//! (task 25686, slice 5 of the proxy-mux programme).
//!
//! Oracle: the §6 criteria — warm trivial exec p50 ≤ 50 ms, 128
//! concurrent requests with zero failures, failure injection with
//! fail/recover ≤ 2 s and no 16–26 s cluster, and matching trip ids
//! on both sides. The record's seam is
//! `doc/2026-11-16T10-40-00Z_design_id5756_multiplexed_streams_seam.md`;
//! the wire contract is `fleet_proxy_proto::mux` (step 1, #13330),
//! terminated proxy-side by `fleet_proxy::mux` (step 2, #15134),
//! originated client-side by `client::mux_roundtrip` (step 3,
//! #15141), with broker trip echo (step 4, #15583).
//!
//! Mode: loopback pair. Both sessions run over one
//! `UnixStream::pair` — the same framed connection machinery the
//! tunnel carries, with the network path factored out. Loopback
//! timings are a regression guard (a p50 that exceeds 50 ms on
//! loopback is broken anywhere); the live-pair thresholds of §6
//! are evaluated by the live run of
//! `scripts/fleet/proxy-mux-acceptance.sh`, never claimed from
//! these tests.
//!
//! Each test prints `MUX_METRIC {...}` lines (hand-rolled JSON,
//! one object per line) that the runner script collects into the
//! acceptance report.

use std::collections::HashMap;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fleet_proxy::mux::{
    MuxConfig, MuxSession, MuxStream, OpenInfo, ResponseHead, StreamError, StreamHandler,
};
use fleet_proxy_proto::mux::{OpenIntent, RequestMeta};

/// Test session tuning: fast keepalive so the pinger path is
/// exercised, a liveness horizon far above any test's runtime so
/// no healthy session is ever declared dead, and the production
/// per-stream ceiling.
fn test_config() -> MuxConfig {
    MuxConfig {
        keepalive: Duration::from_millis(100),
        liveness_timeout: Duration::from_secs(30),
        ceiling: Default::default(),
    }
}

/// The trivial terminating handler: read the request to its fin,
/// then answer `ok` with status 200 and the open's trip id echoed
/// (the step-4 trip echo, #15552). `seen` records every opened
/// stream's trip for the correlation assertion.
fn trivial_handler() -> StreamHandler {
    Arc::new(move |stream: MuxStream, info: OpenInfo| {
        match stream.read_to_fin(1 << 20) {
            Ok(_) => {
                let _ = stream.respond_with(
                    ResponseHead {
                        status: Some(200),
                        transport: Some("mux-acceptance-loopback".to_string()),
                        trip: info.trip.clone(),
                    },
                    b"ok",
                );
            }
            Err(_) => {
                // The originator went away mid-request; nothing to answer.
            }
        }
    })
}

/// A handler that holds every stream open until `release` fires:
/// the in-flight population for the failure-injection test.
fn blocking_handler(release: Arc<Barrier>) -> StreamHandler {
    Arc::new(move |stream: MuxStream, info: OpenInfo| {
        release.wait();
        let _ = stream.respond_with(
            ResponseHead {
                status: Some(200),
                transport: Some("mux-acceptance-loopback".to_string()),
                trip: info.trip.clone(),
            },
            b"ok",
        );
    })
}

/// An originating endpoint never terminates streams: an inbound
/// open violates the direction convention, so reset it on sight
/// (the same shape `client::mux_roundtrip` uses).
fn originator_handler() -> StreamHandler {
    Arc::new(|stream: MuxStream, _info: OpenInfo| {
        stream.reset(fleet_proxy_proto::mux::RST_PROTOCOL_ERROR);
    })
}

struct Loopback {
    client: MuxSession,
    proxy_close: MuxSession,
    proxy_sock: UnixStream,
    client_sock: UnixStream,
    proxy_thread: Option<JoinHandle<()>>,
    reader_thread: Option<JoinHandle<()>>,
    seen: Arc<Mutex<Vec<Option<String>>>>,
}

fn spawn_loopback(handler: StreamHandler) -> Loopback {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (a, b) = UnixStream::pair().expect("loopback pair");
    // Any shutdown lands on the whole socket: the peer's reader
    // sees EOF and its session runs the fail-all pass. The
    // close hooks below make `MuxSession::close` (and
    // `Loopback::join`) shut the transport down, so neither
    // session's `run` can outlive the test: without them a
    // closed-but-connected session's peer blocks forever on a
    // healthy socket (keepalive pings defeat the liveness
    // horizon by design).
    let proxy_sock = b.try_clone().expect("proxy sock clone for kill");
    let client_sock = a.try_clone().expect("client sock clone for kill");
    let proxy_hook_sock = b.try_clone().expect("proxy hook sock clone");
    let client_hook_sock = a.try_clone().expect("client hook sock clone");
    let proxy_reader = b;
    let proxy_writer = proxy_sock.try_clone().expect("proxy writer clone");
    let client_reader = a;
    let client_writer = client_sock.try_clone().expect("client writer clone");

    let proxy = MuxSession::new(
        Box::new(proxy_writer),
        test_config(),
        None,
        Some(Arc::new(move || {
            let _ = proxy_hook_sock.shutdown(Shutdown::Both);
        })),
    );
    let proxy_close = proxy.clone();
    let observed = seen.clone();
    let observed_handler: StreamHandler = Arc::new(move |stream, info| {
        observed.lock().unwrap().push(info.trip.clone());
        handler(stream, info);
    });
    let proxy_thread = thread::spawn(move || proxy.run(proxy_reader, observed_handler));

    let client = MuxSession::new(
        Box::new(client_writer),
        test_config(),
        None,
        Some(Arc::new(move || {
            let _ = client_hook_sock.shutdown(Shutdown::Both);
        })),
    );
    let reader_session = client.clone();
    let reader_thread = thread::spawn(move || reader_session.run(client_reader, originator_handler()));

    Loopback {
        client,
        proxy_close,
        proxy_sock,
        client_sock,
        proxy_thread: Some(proxy_thread),
        reader_thread: Some(reader_thread),
        seen,
    }
}

impl Loopback {
    /// Simulate tunnel death the way the record's failure path
    /// detects it: transport shutdown, so both readers see EOF and
    /// every in-flight stream fails with its trip id preserved.
    fn kill_tunnel(&self) {
        let _ = self.proxy_sock.shutdown(Shutdown::Both);
        let _ = self.client_sock.shutdown(Shutdown::Both);
        self.proxy_close.close();
        self.client.close();
    }

    fn join(mut self) {
        self.client.close();
        if let Some(t) = self.proxy_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.reader_thread.take() {
            let _ = t.join();
        }
    }
}

/// One trivial exec over the mux: open a stream with `trip`, send
/// a small body, read the `ok` to its fin. Returns the roundtrip
/// latency and the trip id the terminator echoed.
fn trivial_roundtrip(client: &MuxSession, trip: Option<String>) -> (Duration, Option<String>) {
    let start = Instant::now();
    let stream = client
        .open_stream_with(
            4799,
            &OpenIntent::Request {
                method: "GET".to_string(),
                path: "/".to_string(),
            },
            trip,
            RequestMeta::default(),
        )
        .expect("open stream");
    stream.send_chunk(b"ping").expect("send trivial body");
    stream.close_send().expect("close request direction");
    let body = stream.read_to_fin(1 << 20).expect("read trivial response");
    assert_eq!(body, b"ok", "trivial response body");
    assert_eq!(stream.status(), Some(200), "trivial response status");
    (start.elapsed(), stream.echoed_trip())
}

fn percentile(mut samples: Vec<u128>, pct: f64) -> u128 {
    assert!(!samples.is_empty());
    samples.sort_unstable();
    let rank = ((pct / 100.0) * (samples.len() as f64)).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

fn metric(name: &str, fields: &str) {
    println!("MUX_METRIC {{\"name\":\"{name}\",{fields}}}");
}

#[test]
fn acceptance_warm_trivial_exec_p50() {
    // Oracle: #5756 §6 — warm trivial exec trip p50 ≤ 50 ms.
    // First roundtrip warms the pair and is excluded; the p50 is
    // over the following 50 warm roundtrips.
    let pair = spawn_loopback(trivial_handler());
    let seen = pair.seen.clone();
    let _ = trivial_roundtrip(&pair.client, Some("warmup".to_string()));
    let mut samples = Vec::with_capacity(50);
    for i in 0..50 {
        let (elapsed, echo) =
            trivial_roundtrip(&pair.client, Some(format!("warm-{i:02}")));
        assert_eq!(echo.as_deref(), Some(format!("warm-{i:02}").as_str()));
        samples.push(elapsed.as_micros());
    }
    let p50 = percentile(samples.clone(), 50.0);
    let max = samples.iter().max().copied().unwrap_or(0);
    metric(
        "warm_trivial_exec",
        &format!("\"n\":50,\"p50_us\":{p50},\"max_us\":{max},\"threshold_us\":50000"),
    );
    assert!(
        p50 <= 50_000,
        "warm trivial exec p50 {p50}µs exceeds the §6 50 ms bound even on loopback"
    );
    assert_eq!(seen.lock().unwrap().len(), 51, "every open reached the terminator");
    pair.join();
}

#[test]
fn acceptance_concurrent_128_zero_failures() {
    // Oracle: #5756 §6 — 128 concurrent requests over the single
    // tunnel: 0 failures, every trip echoed back to its caller.
    let pair = spawn_loopback(trivial_handler());
    let client = pair.client.clone();
    let start = Instant::now();
    let handles: Vec<JoinHandle<(String, Duration, Option<String>)>> = (0..128)
        .map(|i| {
            let client = client.clone();
            thread::spawn(move || {
                let trip = format!("conc-{i:03}");
                let (elapsed, echo) = trivial_roundtrip(&client, Some(trip.clone()));
                (trip, elapsed, echo)
            })
        })
        .collect();
    let mut failures = 0;
    let mut mismatches = 0;
    let mut samples = Vec::with_capacity(128);
    for h in handles {
        match h.join() {
            Ok((trip, elapsed, echo)) => {
                samples.push(elapsed.as_micros());
                if echo.as_deref() != Some(trip.as_str()) {
                    mismatches += 1;
                }
            }
            Err(_) => failures += 1,
        }
    }
    let wall_ms = start.elapsed().as_millis();
    let p50 = percentile(samples, 50.0);
    metric(
        "concurrent_128",
        &format!("\"n\":128,\"failures\":{failures},\"trip_mismatches\":{mismatches},\"wall_ms\":{wall_ms},\"p50_us\":{p50},\"baseline_wall_ms\":14600"),
    );
    assert_eq!(failures, 0, "128 concurrent: no request may fail");
    assert_eq!(mismatches, 0, "128 concurrent: every trip must echo to its caller");
    assert!(
        wall_ms <= 14_600,
        "128 concurrent wall {wall_ms}ms exceeds the §6 14.6 s ramp baseline"
    );
    pair.join();
}

#[test]
fn acceptance_failure_recovery_within_2s() {
    // Oracle: #5756 §6 — kill the tunnel with streams in flight:
    // every in-flight call fails fast (≤ 2 s, never the 16–26 s
    // cluster) with its own trip id preserved, and a fresh tunnel
    // serves again immediately after.
    let release = Arc::new(Barrier::new(9));
    let pair = spawn_loopback(blocking_handler(release.clone()));
    // Eight streams, held in flight by the blocking terminator.
    let client = pair.client.clone();
    let handles: Vec<JoinHandle<(String, StreamError)>> = (0..8)
        .map(|i| {
            let client = client.clone();
            thread::spawn(move || {
                let trip = format!("fail-{i}");
                let stream = client
                    .open_stream_with(
                        4799,
                        &OpenIntent::Request {
                            method: "GET".to_string(),
                            path: "/".to_string(),
                        },
                        Some(trip.clone()),
                        RequestMeta::default(),
                    )
                    .expect("open stream");
                stream.send_chunk(b"ping").expect("send");
                stream.close_send().expect("close send");
                match stream.read_to_fin(1 << 20) {
                    Ok(_) => (trip, StreamError::Reset { reason: "unexpected-ok".to_string() }),
                    Err(e) => (trip, e),
                }
            })
        })
        .collect();
    // Wait until all eight streams have reached the terminator
    // and are blocked there (bounded: a missing open is a test
    // failure, not a hang), so the kill lands on a genuinely
    // in-flight population.
    let waited = Instant::now();
    loop {
        if pair.seen.lock().unwrap().len() >= 8 {
            break;
        }
        assert!(
            waited.elapsed() < Duration::from_secs(10),
            "terminator never saw all 8 in-flight streams"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let kill_at = Instant::now();
    pair.kill_tunnel();
    // Release the blocked terminator handlers (the 9th barrier
    // party): their responses fail silently on the dead session
    // and their threads exit cleanly for the join below.
    release.wait();
    let mut slow = 0;
    let mut wrong_trip = 0;
    let mut unexpected = 0;
    for h in handles {
        let (trip, err) = h.join().expect("client thread");
        match err {
            StreamError::TunnelLost { trip: got } => {
                if got.as_deref() != Some(trip.as_str()) {
                    wrong_trip += 1;
                }
            }
            _ => unexpected += 1,
        }
        if kill_at.elapsed() > Duration::from_secs(2) {
            slow += 1;
        }
    }
    let fail_elapsed_ms = kill_at.elapsed().as_millis();
    // Fresh tunnel serves again: the single-flight reconnect half.
    let pair2 = spawn_loopback(trivial_handler());
    let (elapsed, echo) = trivial_roundtrip(&pair2.client, Some("after-recovery".to_string()));
    let recovered = echo.as_deref() == Some("after-recovery");
    let total_ms = kill_at.elapsed().as_millis();
    metric(
        "failure_recovery",
        &format!("\"inflight\":8,\"fail_elapsed_ms\":{fail_elapsed_ms},\"slow\":{slow},\"wrong_trip\":{wrong_trip},\"unexpected\":{unexpected},\"recovered\":{recovered},\"recover_roundtrip_us\":{},\"total_ms\":{total_ms},\"threshold_ms\":2000", elapsed.as_micros()),
    );
    assert_eq!(unexpected, 0, "in-flight streams must fail as tunnel-lost, not otherwise");
    assert_eq!(wrong_trip, 0, "every failed stream keeps its own trip id");
    assert_eq!(slow, 0, "fail-fast must land within the §6 2 s bound");
    assert!(recovered, "a fresh tunnel must serve again after the kill");
    assert!(
        total_ms <= 2000,
        "kill-to-recovered {total_ms}ms exceeds the §6 2 s bound"
    );
    pair.join();
    pair2.join();
}

#[test]
fn acceptance_trip_correlation_both_sides() {
    // Oracle: #5756 §6 item 4 — trip logs on both sides keep the
    // same trip id per call, so the client/daemon split stays
    // measurable. Includes one tripless stream: absence must
    // correlate as absence, never as a foreign id.
    let pair = spawn_loopback(trivial_handler());
    let seen = pair.seen.clone();
    let mut sent: Vec<Option<String>> = (0..16).map(|i| Some(format!("trip-{i:02}"))).collect();
    sent.push(None);
    let mut echoed = Vec::with_capacity(sent.len());
    for trip in &sent {
        let (_, echo) = trivial_roundtrip(&pair.client, trip.clone());
        echoed.push(echo);
    }
    let mut seen_sorted = seen.lock().unwrap().clone();
    let mut sent_sorted = sent.clone();
    let mut echoed_sorted = echoed.clone();
    seen_sorted.sort();
    sent_sorted.sort();
    echoed_sorted.sort();
    let matched = seen_sorted == sent_sorted && echoed_sorted == sent_sorted;
    metric(
        "trip_correlation",
        &format!("\"n\":{},\"matched\":{matched}", sent.len()),
    );
    assert_eq!(seen_sorted, sent_sorted, "terminator saw exactly the trips sent");
    assert_eq!(echoed_sorted, sent_sorted, "every echo matches the trip sent");
    // The tripless stream correlates as absence on both sides.
    let mut counts: HashMap<Option<String>, usize> = HashMap::new();
    for t in &echoed {
        *counts.entry(t.clone()).or_default() += 1;
    }
    assert_eq!(counts.get(&None), Some(&1), "exactly one tripless echo");
    pair.join();
}

#[test]
fn harness_lists_its_acceptance_tests() {
    // The runner asserts these exact names before dispatch: a
    // zero-test filter pass must never count as acceptance.
    let names = [
        "acceptance_warm_trivial_exec_p50",
        "acceptance_concurrent_128_zero_failures",
        "acceptance_failure_recovery_within_2s",
        "acceptance_trip_correlation_both_sides",
    ];
    for name in names {
        metric("test_present", &format!("\"test\":\"{name}\""));
    }
}
