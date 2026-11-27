//! Repeatable offline acceptance probe for the complete mux framing path.
//!
//! Run explicitly with:
//! `cargo test -p fleet-proxy --test mux_acceptance -- --ignored --nocapture`
//!
//! The peer is an in-process terminating handler, not a laptop/runtime-proxy
//! pair. These measurements exercise framing, concurrent stream dispatch,
//! tunnel failure propagation, and trip echo, but MUST NOT be reported as
//! satisfying #5756's live latency/recovery thresholds. Run a live paired
//! probe separately when the endpoints are reachable.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use fleet_proxy::mux::{
    MuxConfig, MuxSession, MuxStream, OpenInfo, ResponseHead, StreamError, StreamHandler,
};
use fleet_proxy_proto::mux::{OpenIntent, StreamCeiling};

const REQUESTS: usize = 128;
const WARM_SAMPLES: usize = 200;

fn config() -> MuxConfig {
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

fn linked(handler: StreamHandler) -> (MuxSession, UnixStream) {
    let (left, right) = UnixStream::pair().unwrap();
    let kill = left.try_clone().unwrap();
    let client = MuxSession::new(
        Box::new(left.try_clone().unwrap()),
        config(),
        None,
        Some(shutdown_hook(&left)),
    );
    let broker = MuxSession::new(
        Box::new(right.try_clone().unwrap()),
        config(),
        None,
        Some(shutdown_hook(&right)),
    );
    let broker_run = broker.clone();
    thread::spawn(move || broker_run.run(right, handler));
    let client_run = client.clone();
    thread::spawn(move || {
        let reject: StreamHandler = Arc::new(|stream, _| {
            stream.reset(fleet_proxy_proto::mux::RST_PROTOCOL_ERROR);
        });
        client_run.run(left, reject);
    });
    (client, kill)
}

fn request(session: &MuxSession, trip: String) -> Result<(Duration, String), StreamError> {
    let start = Instant::now();
    let stream = session.open_stream(
        4801,
        &OpenIntent::Request {
            method: "POST".to_string(),
            path: "/exec".to_string(),
        },
        Some(trip.clone()),
    )?;
    stream.send_chunk(b"{}")?;
    stream.close_send()?;
    let response = stream.read_to_fin(1024)?;
    if response != b"{}" || stream.status() != Some(200) {
        return Err(StreamError::Protocol(
            "unexpected acceptance response".into(),
        ));
    }
    if stream.echoed_trip().as_deref() != Some(trip.as_str()) {
        return Err(StreamError::Protocol("trip id echo mismatch".into()));
    }
    Ok((start.elapsed(), trip))
}

fn percentile_50(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

/// Prints machine-readable, explicitly offline evidence for the four #5756
/// acceptance dimensions. The performance targets are not asserted here:
/// loopback/in-process timings are not comparable to a live proxy pair.
#[test]
#[ignore = "measurement probe; run explicitly with --ignored --nocapture"]
fn proxy_mux_acceptance_probe() {
    let daemon_trips = Arc::new(Mutex::new(Vec::new()));
    let seen = daemon_trips.clone();
    let handler: StreamHandler = Arc::new(move |stream: MuxStream, info: OpenInfo| {
        if let Ok(body) = stream.read_to_fin(1024) {
            if let Some(trip) = info.trip.clone() {
                seen.lock().unwrap().push(trip.clone());
            }
            let _ = stream.respond_with(
                ResponseHead {
                    status: Some(200),
                    transport: None,
                    trip: info.trip,
                },
                &body,
            );
        }
    });
    let (client, _kill) = linked(handler);

    // Warm trivial exec-shaped request samples on one persistent session.
    let mut warm_us = Vec::with_capacity(WARM_SAMPLES);
    for n in 0..WARM_SAMPLES {
        let (elapsed, _) =
            request(&client, format!("warm-{n}")).expect("warm mux request succeeds");
        warm_us.push(elapsed.as_micros());
    }
    let warm_p50_us = percentile_50(&mut warm_us);

    // Concurrent requests share the same mux session/tunnel.
    let concurrency_started = Instant::now();
    let mut workers = Vec::with_capacity(REQUESTS);
    for n in 0..REQUESTS {
        let session = client.clone();
        workers.push(thread::spawn(move || {
            request(&session, format!("concurrent-{n}"))
        }));
    }
    let mut failures = Vec::new();
    for worker in workers {
        if let Err(error) = worker.join().expect("probe worker did not panic") {
            failures.push(error.to_string());
        }
    }
    let concurrency_wall_ms = concurrency_started.elapsed().as_millis();
    assert!(failures.is_empty(), "concurrent mux failures: {failures:?}");

    // Inject a tunnel break during an in-flight stream and time fail-fast.
    let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
    let stalled_handler: StreamHandler = Arc::new(move |stream: MuxStream, _info: OpenInfo| {
        let _ = waiting_tx.send(());
        let _ = stream.read_to_fin(1024);
        thread::sleep(Duration::from_secs(10));
    });
    let (failure_client, kill) = linked(stalled_handler);
    let request_session = failure_client.clone();
    let request_thread = thread::spawn(move || {
        let stream = request_session
            .open_stream(
                4801,
                &OpenIntent::Request {
                    method: "POST".into(),
                    path: "/exec".into(),
                },
                Some("failure-injection".into()),
            )
            .unwrap();
        stream.send_chunk(b"{}").unwrap();
        stream.close_send().unwrap();
        stream.recv_chunk(Duration::from_secs(15))
    });
    waiting_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("handler received failure injection request");
    let fail_started = Instant::now();
    kill.shutdown(std::net::Shutdown::Both).unwrap();
    let failure = request_thread.join().expect("failure worker did not panic");
    let failure_recovery_ms = fail_started.elapsed().as_millis();
    assert!(failure.is_err(), "killed tunnel unexpectedly returned data");
    assert!(
        failure_recovery_ms <= 2_000,
        "dead tunnel took {failure_recovery_ms}ms to fail"
    );

    // The server-observed trip IDs must match the request-side set exactly.
    let client_trips: BTreeSet<String> = (0..WARM_SAMPLES)
        .map(|n| format!("warm-{n}"))
        .chain((0..REQUESTS).map(|n| format!("concurrent-{n}")))
        .collect();
    let broker_trips: BTreeSet<String> = daemon_trips.lock().unwrap().iter().cloned().collect();
    assert_eq!(
        broker_trips, client_trips,
        "client/broker trip IDs diverged"
    );

    let sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    println!(
        "MUX_ACCEPTANCE_REPORT={}",
        serde_json::json!({
            "issue": 5756,
            "sha": sha,
            "environment": "offline in-process UnixStream pair; synthetic /exec handler; one mux session",
            "load": { "warm_samples": WARM_SAMPLES, "concurrent_requests": REQUESTS },
            "warm_trivial_exec_p50_us": warm_p50_us,
            "concurrent_128": { "failures": failures.len(), "wall_ms": concurrency_wall_ms },
            "failure_injection": { "result": "tunnel-kill", "recovery_ms": failure_recovery_ms },
            "trip_correlation": { "client_ids": client_trips.len(), "broker_ids": broker_trips.len(), "match": true },
            "live_acceptance": "UNVERIFIED; offline measurements do not satisfy live thresholds"
        })
    );
}
