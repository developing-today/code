//! Live native-against-hosted bridge test (#3818).
//!
//! The last open acceptance item of the bridge design record: the
//! real Rust client (`BridgeHttpClient` + `BridgeDialer`) against
//! the real hosted worker (`UserDO` via the test-only harness
//! under `wrangler dev --local`), not the mirrors the unit tests
//! run against. The live hosted worker is the oracle here: the
//! mirror-based unit tests in `bridge.rs` / `bridge_http.rs` pin
//! the protocol, but only this test shows the two implementations
//! converging over real HTTP and a real WebSocket upgrade.
//!
//! Gated: runs only with `BRIDGE_LIVE_TEST=1`. Without it the test
//! prints why it skipped and passes, so default `cargo test` and
//! unrelated CI are unaffected. `wrangler` resolves from
//! `BRIDGE_LIVE_WRANGLER`, else `wrangler` on `PATH`; when neither
//! resolves the test skips with a clear message instead of failing
//! the suite for a missing binary. When wrangler spawns but never
//! answers, that is a failure: the harness is part of the slice.
//!
//! Shape: two native clients share one binding and one ID key.
//! Client A pushes its chain, client B pushes its chain, both over
//! the live HTTPS legs; a fresh store replays both chains from an
//! empty pull (byte-identical events — the original signatures,
//! never re-signed); then client A opens a live WebSocket session
//! carrying one more locally-created event to the hosted store and
//! receiving B's chain back. Convergence is asserted both
//! directions, and every event the test reads back is compared
//! byte-for-byte with the bytes its author signed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::SigningKey;
use p2panda_core::{Hash, SigningKey as PandaSigningKey};

use id_core::waiting::WaitingLimits;
use id_events::event_store::EventStore;
use id_store::chain::{ChainExt, decode, sign};
use id_store::range::Id;
use id_sync::bridge::{BridgeDialer, BridgeStore, connect};
use id_sync::bridge_http::{BridgeHttpClient, PushOutcome, ReqwestHttp};

const NOW_MS_FALLBACK: u64 = 0;
const READY_TIMEOUT_SECS: u64 = 120;
const SESSION_TIMEOUT_SECS: u64 = 120;
const AUTHOR_A_SEED: u8 = 7;
const AUTHOR_B_SEED: u8 = 8;

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(NOW_MS_FALLBACK)
}

fn message_ext() -> ChainExt {
    ChainExt {
        kind: "message".to_owned(),
        group: None,
        after: vec![],
        tag: None,
        document: None,
        removes: None,
        note: None,
        link: None,
        checkpoint: None,
        rotation: None,
        chat: None,
        anchor: None,
    }
}

/// A signed chain of `count` events by the author with `seed`,
/// starting at `start_seq` after `backlink` (the bridge.rs unit
/// fixtures' shape, extended so a live store can grow its chain
/// after an earlier push).
fn make_events_from(seed: u8, start_seq: u64, backlink: Option<Hash>, count: u64) -> Vec<Vec<u8>> {
    let key = PandaSigningKey::from_bytes(&[seed; 32]);
    let mut out = Vec::new();
    let mut backlink = backlink;
    for seq in start_seq..start_seq + count {
        let event = sign(
            &key,
            p2panda_core::SeqNum::try_from(seq).unwrap(),
            backlink,
            None,
            message_ext(),
        )
        .unwrap();
        backlink = Some(event.id);
        out.push(event.bytes);
    }
    out
}

fn make_events(seed: u8, count: u64) -> Vec<Vec<u8>> {
    make_events_from(seed, 0, None, count)
}

fn author_hex(seed: u8) -> String {
    let key = PandaSigningKey::from_bytes(&[seed; 32]);
    hex_encode(key.verifying_key().as_bytes())
}

fn open_store(now_ms: u64) -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), now_ms).unwrap();
    (dir, store)
}

/// One event id to the exact bytes its author signed: the
/// signature-preservation oracle for everything the test reads
/// back from the hosted store.
fn originals(events: &[Vec<u8>]) -> HashMap<String, Vec<u8>> {
    let mut map = HashMap::new();
    for bytes in events {
        let event = decode(bytes).unwrap();
        map.insert(hex_encode(event.id.as_bytes()), bytes.clone());
    }
    map
}

fn assert_byte_identical(store: &EventStore, expected: &HashMap<String, Vec<u8>>) {
    for (id_hex, bytes) in expected {
        let mut id: Id = [0; 32];
        for (index, byte) in id.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&id_hex[index * 2..index * 2 + 2], 16).unwrap();
        }
        let held = store.bridge_event_bytes(&id).unwrap();
        assert_eq!(
            held.as_ref(),
            Some(bytes),
            "event {id_hex} is not byte-identical to the signed original"
        );
    }
}

/// The live hosted worker under `wrangler dev --local`, killed on
/// drop even when the test fails.
struct LiveWorker {
    child: Child,
    port: u16,
    log_path: PathBuf,
}

impl LiveWorker {
    /// Resolves wrangler and spawns the harness. Returns `None`
    /// (after printing why) when the test cannot run in this
    /// environment: the gate is off, or no wrangler resolves.
    fn start() -> Option<Self> {
        if std::env::var_os("BRIDGE_LIVE_TEST").as_deref() != Some(std::ffi::OsStr::new("1")) {
            println!("skipping live bridge test: set BRIDGE_LIVE_TEST=1 to run it");
            return None;
        }
        let wrangler = match std::env::var("BRIDGE_LIVE_WRANGLER") {
            Ok(path) if !path.is_empty() => path,
            _ => {
                let resolves = Command::new("wrangler")
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success());
                if !resolves {
                    println!(
                        "skipping live bridge test: no wrangler found (set BRIDGE_LIVE_WRANGLER or put wrangler on PATH)"
                    );
                    return None;
                }
                "wrangler".to_owned()
            }
        };
        let hosted_worker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../hosted-worker");
        if !hosted_worker.join("wrangler.test.toml").is_file() {
            println!(
                "skipping live bridge test: no hosted worker at {}",
                hosted_worker.display()
            );
            return None;
        }
        let port = TcpListener::bind("127.0.0.1:0")
            .ok()
            .and_then(|listener| listener.local_addr().ok())
            .map(|addr| addr.port());
        let Some(port) = port else {
            println!("skipping live bridge test: no free loopback port");
            return None;
        };
        let log_path = std::env::temp_dir().join(format!("bridge-live-wrangler-{port}.log"));
        let log = std::fs::File::create(&log_path).ok();
        let (stdout, stderr) = match log {
            Some(file) => match file.try_clone() {
                Ok(clone) => (Stdio::from(file), Stdio::from(clone)),
                Err(_) => (Stdio::null(), Stdio::null()),
            },
            None => (Stdio::null(), Stdio::null()),
        };
        let child = Command::new(&wrangler)
            .args([
                "dev",
                "-c",
                "wrangler.test.toml",
                "--port",
                &port.to_string(),
                "--local",
            ])
            .current_dir(&hosted_worker)
            .stdout(stdout)
            .stderr(stderr)
            .spawn();
        match child {
            Ok(child) => {
                println!(
                    "live bridge test: wrangler dev on 127.0.0.1:{port} (log {})",
                    log_path.display()
                );
                Some(Self {
                    child,
                    port,
                    log_path,
                })
            }
            Err(err) => {
                println!("skipping live bridge test: wrangler did not spawn: {err}");
                None
            }
        }
    }

    /// Waits until the harness answers a challenge through the
    /// path prefix, or panics: wrangler started, so readiness is
    /// part of the slice, not the environment.
    async fn wait_ready(&self, base: &str) {
        let probe = reqwest::Client::new();
        let url = format!("{base}/v1/bridge/challenge");
        for _ in 0..READY_TIMEOUT_SECS {
            if let Ok(response) = probe.get(&url).send().await {
                if response.status().is_success() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        panic!(
            "wrangler dev on 127.0.0.1:{} never answered {url} (log {})\n{}",
            self.port,
            self.log_path.display(),
            log_tail(&self.log_path),
        );
    }
}

fn log_tail(path: &std::path::Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(3000);
            format!(
                "wrangler log tail:\n{}",
                String::from_utf8_lossy(&bytes[start..])
            )
        }
        Err(err) => format!("wrangler log unreadable: {err}"),
    }
}

impl Drop for LiveWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn live_native_against_hosted_bridge_converges_both_directions() {
    let Some(worker) = LiveWorker::start() else {
        return;
    };
    // One fresh identity per run: the harness persists DO state
    // under .wrangler/state, and a binding binds immutably on
    // first use, so a reused binding would collide with an
    // earlier run's object.
    let id_key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
    let binding = hex_encode(&rand::random::<[u8; 16]>());
    let iroh_hex = hex_encode(id_key.verifying_key().as_bytes());
    let base = format!(
        "http://127.0.0.1:{}/__harness/{binding}/{binding}/{iroh_hex}/us",
        worker.port
    );
    let ws_url = format!("{base}/v1/bridge/ws");
    worker.wait_ready(&base).await;

    let now = now_ms();
    let a_events = make_events(AUTHOR_A_SEED, 3);
    let b_events = make_events(AUTHOR_B_SEED, 3);
    let (_dir_a, mut store_a) = open_store(now);
    let (_dir_b, mut store_b) = open_store(now);
    store_a.bridge_ingest(&a_events, now).unwrap();
    store_b.bridge_ingest(&b_events, now).unwrap();

    let mut http = ReqwestHttp::new().unwrap();
    let client_a = BridgeHttpClient::new(binding.clone(), id_key.clone(), base.clone());
    let client_b = BridgeHttpClient::new(binding.clone(), id_key.clone(), base.clone());

    // Native -> hosted over the live HTTPS legs: both clients push.
    let pushed_a = client_a.push(&mut http, &a_events).await.unwrap();
    assert_eq!(pushed_a.outcomes.len(), 3);
    assert!(
        pushed_a
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, PushOutcome::Accepted { .. })),
        "client A push outcomes: {:?}",
        pushed_a.outcomes
    );
    let pushed_b = client_b.push(&mut http, &b_events).await.unwrap();
    assert_eq!(pushed_b.outcomes.len(), 3);
    assert!(
        pushed_b
            .outcomes
            .iter()
            .all(|outcome| matches!(outcome, PushOutcome::Accepted { .. })),
        "client B push outcomes: {:?}",
        pushed_b.outcomes
    );
    let heads = client_a.heads(&mut http).await.unwrap();
    assert_eq!(heads.len(), 2);
    for (seed, next_seq) in [(AUTHOR_A_SEED, 3), (AUTHOR_B_SEED, 3)] {
        let head = heads
            .iter()
            .find(|head| head.author == author_hex(seed))
            .unwrap_or_else(|| panic!("no hosted head for author {seed}"));
        assert_eq!(head.next_seq, next_seq);
    }

    // Replay into an empty store over the live pull leg: every
    // event arrives byte-identical to what its author signed.
    let mut all = a_events.clone();
    all.extend(b_events.iter().cloned());
    let expected = originals(&all);
    let (_dir_e, mut store_e) = open_store(now);
    let pulled = client_a.pull(&mut http, &[], None).await.unwrap();
    assert!(!pulled.more);
    assert_eq!(pulled.events.len(), 6);
    let pulled_originals = originals(&pulled.events);
    assert_eq!(
        pulled_originals, expected,
        "the live pull did not return exactly the signed originals"
    );
    store_e.bridge_ingest(&pulled.events, now).unwrap();
    assert_eq!(store_e.bridge_sync_entries().unwrap().len(), 6);
    assert_byte_identical(&store_e, &expected);

    // One more event by author A, created locally after the push:
    // the WebSocket session must carry it to the hosted store,
    // and bring B's chain back the other way.
    let a_last = decode(&a_events[2]).unwrap();
    let a_extra = make_events_from(AUTHOR_A_SEED, 3, Some(a_last.id), 1);
    store_a.bridge_ingest(&a_extra, now).unwrap();
    let mut expected_after = expected.clone();
    expected_after.extend(originals(&a_extra));

    let mut dialer = BridgeDialer::new(binding.clone(), id_key.clone());
    // The connect error carries its cause chain: an upgrade
    // refusal (the harness or the DO answering 4xx) reads very
    // differently from a dropped connection, and the next failure
    // needs that distinction.
    let mut transport =
        match tokio::time::timeout(Duration::from_secs(SESSION_TIMEOUT_SECS), connect(&ws_url))
            .await
        {
            Ok(Ok(transport)) => transport,
            Ok(Err(err)) => panic!("live bridge ws connect failed: {err:#}"),
            Err(_) => panic!("live bridge ws connect timed out after {SESSION_TIMEOUT_SECS}s"),
        };
    let result = tokio::time::timeout(
        Duration::from_secs(SESSION_TIMEOUT_SECS),
        dialer.sync(&mut store_a, &mut transport, now),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        result.withholding.is_empty(),
        "live session withholding: {:?}",
        result.withholding
    );
    assert!(result.token.is_some());
    assert_eq!(result.acks.iter().map(|ack| ack.received).sum::<u64>(), 3);
    assert_eq!(result.acks.iter().map(|ack| ack.refused).sum::<u64>(), 0);
    assert_eq!(store_a.bridge_sync_entries().unwrap().len(), 7);
    assert_byte_identical(&store_a, &expected_after);

    // The hosted side converged too: it holds A's fourth event,
    // which only ever travelled over the WebSocket session.
    let heads_after = client_a.heads(&mut http).await.unwrap();
    let head_a = heads_after
        .iter()
        .find(|head| head.author == author_hex(AUTHOR_A_SEED))
        .unwrap();
    assert_eq!(head_a.next_seq, 4);
    let head_b = heads_after
        .iter()
        .find(|head| head.author == author_hex(AUTHOR_B_SEED))
        .unwrap();
    assert_eq!(head_b.next_seq, 3);
}
