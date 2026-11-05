//! The #6109 spike proof harness: media-shaped UDP bursts,
//! direct vs through the CONNECT-UDP proxy, against a loopback
//! echo target. Not a test — a measurement. Run it with:
//!
//! ```sh
//! cargo run -p fleet-proxy --example connect_udp_proof
//! ```
//!
//! It bursts 1200-byte datagrams at ~90/s for 30 s each way
//! (the Moonlight media shape: small datagrams, steady rate),
//! once over a direct connected UDP socket and once through
//! the proxy, and prints loss % plus RTT percentiles for both.
//! Loopback is deliberately the *flattering* environment for
//! the TCP carrier (no wire loss to convert into retransmit
//! stalls); the point is a reproducible baseline, and the
//! numbers land on #6109 next to that caveat.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fleet_proxy::connect_udp::{self, UdpProxyClient};

const RATE_PER_SEC: u64 = 90;
const BURST_SECS: u64 = 30;
const PAYLOAD_BYTES: usize = 1200;
const GRACE: Duration = Duration::from_secs(3);

struct Stats {
    sent: u64,
    received: u64,
    rtt: Vec<Duration>,
}

impl Stats {
    fn report(&self, label: &str) {
        let loss = self.sent - self.received;
        let loss_pct = if self.sent == 0 {
            0.0
        } else {
            loss as f64 * 100.0 / self.sent as f64
        };
        let mut sorted = self.rtt.clone();
        sorted.sort();
        let pct = |p: f64| -> Duration {
            if sorted.is_empty() {
                return Duration::ZERO;
            }
            let idx = ((sorted.len() as f64 * p) as usize).min(sorted.len() - 1);
            sorted[idx]
        };
        println!(
            "{label}: sent={} received={} loss={} ({:.3}%) rtt p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms",
            self.sent,
            self.received,
            loss,
            loss_pct,
            pct(0.50).as_secs_f64() * 1000.0,
            pct(0.95).as_secs_f64() * 1000.0,
            pct(0.99).as_secs_f64() * 1000.0,
            pct(1.0).as_secs_f64() * 1000.0,
        );
    }
}

/// One burst: `send` ships one datagram; `recv` yields reply
/// datagrams (with a short timeout when idle). Sender paces on
/// a fixed schedule; the receiver drains until the grace period
/// after the last send.
fn burst(
    mut send: impl FnMut(&[u8]) -> std::io::Result<()>,
    mut recv: impl FnMut() -> std::io::Result<Vec<u8>> + Send + 'static,
) -> Stats {
    let total = RATE_PER_SEC * BURST_SECS;
    let interval = Duration::from_micros(1_000_000 / RATE_PER_SEC);
    let send_times: Arc<Mutex<HashMap<u64, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
    let done_sending = Arc::new(AtomicBool::new(false));

    let st = Arc::clone(&send_times);
    let ds = Arc::clone(&done_sending);
    let receiver = std::thread::spawn(move || {
        let mut rtt = Vec::new();
        let mut received = 0u64;
        let deadline = Instant::now() + Duration::from_secs(BURST_SECS) + GRACE;
        loop {
            match recv() {
                Ok(d) => {
                    received += 1;
                    if d.len() >= 8 {
                        let seq = u64::from_be_bytes(d[..8].try_into().unwrap());
                        if let Some(t0) = st.lock().unwrap().get(&seq) {
                            rtt.push(t0.elapsed());
                        }
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if ds.load(Ordering::SeqCst) && Instant::now() > deadline {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        (received, rtt)
    });

    let start = Instant::now();
    let mut sent = 0u64;
    for seq in 0..total {
        let at = start + interval * (seq as u32);
        let now = Instant::now();
        if at > now {
            std::thread::sleep(at - now);
        }
        let mut payload = vec![0u8; PAYLOAD_BYTES];
        payload[..8].copy_from_slice(&seq.to_be_bytes());
        for (i, b) in payload[8..].iter_mut().enumerate() {
            *b = ((seq + i as u64) % 251) as u8;
        }
        send_times.lock().unwrap().insert(seq, Instant::now());
        if send(&payload).is_ok() {
            sent += 1;
        }
    }
    done_sending.store(true, Ordering::SeqCst);
    let (received, rtt) = receiver.join().unwrap_or((0, Vec::new()));
    Stats {
        sent,
        received,
        rtt,
    }
}

fn main() {
    // The echo target.
    let echo = UdpSocket::bind("127.0.0.1:0").expect("bind echo");
    let echo_addr = echo.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; connect_udp::MAX_DATAGRAM_BYTES];
        loop {
            match echo.recv_from(&mut buf) {
                Ok((n, from)) => {
                    let _ = echo.send_to(&buf[..n], from);
                }
                Err(_) => break,
            }
        }
    });

    // Baseline: direct connected UDP to the echo target.
    let direct = UdpSocket::bind("127.0.0.1:0").expect("bind direct");
    direct.connect(echo_addr).expect("connect direct");
    direct
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let direct_recv = direct.try_clone().unwrap();
    let baseline = burst(
        |p| direct.send(p).map(|_| ()),
        move || {
            let mut buf = vec![0u8; connect_udp::MAX_DATAGRAM_BYTES];
            direct_recv.recv(&mut buf).map(|n| buf[..n].to_vec())
        },
    );
    baseline.report("direct UDP ");

    // Through the proxy: the capability under test.
    let listener = connect_udp::listen("127.0.0.1:0").expect("bind proxy");
    let proxy_addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || connect_udp::serve(listener));
    let client = UdpProxyClient::connect(&proxy_addr, "127.0.0.1", echo_addr.port())
        .expect("CONNECT-UDP handshake");
    let (mut tx, mut rx) = client.split();
    rx.set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let proxied = burst(|p| tx.send(p), move || rx.recv());
    proxied.report("via proxy  ");

    let added = |a: &Stats, b: &Stats| {
        let med = |s: &Stats| -> f64 {
            let mut v = s.rtt.clone();
            v.sort();
            if v.is_empty() {
                0.0
            } else {
                v[v.len() / 2].as_secs_f64() * 1000.0
            }
        };
        med(a) - med(b)
    };
    println!(
        "added latency at p50: {:.2}ms (proxy median minus direct median)",
        added(&proxied, &baseline)
    );
}
