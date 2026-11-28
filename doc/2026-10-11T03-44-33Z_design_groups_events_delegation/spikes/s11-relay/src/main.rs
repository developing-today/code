//! S11 relay driver. Runs the stock `iroh-relay` 1.3.0 binary and drives iroh 1.3.0 endpoints
//! against it on loopback. Endpoints have no IP transport, so every packet crosses a relay.
//!
//!   s11 restart <TERM|KILL|INT> <runs> <down_ms>   scenario 1: restart the relay under two peers
//!   s11 idle                                       scenario 4: non-home and home relay idle
//!   s11 dial <relay_url> [--ca <der>]... [--token <t>] [--timeout-s <n>]   scenarios 2 and 5

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    net::{SocketAddr, TcpStream},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::{Child, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr, RelayConfig, RelayMode, RelayUrl, Watcher,
    endpoint::{Connection, RecvStream, SendStream, presets::Minimal},
    tls::CaTlsConfig,
};
use rustls_pki_types::CertificateDer;
use serde_json::{Map, Value, json};
use tokio::time::{interval, sleep, timeout};
use tracing::info;

const RELAY_BIN: &str = "/home/user/work/spikes/tools/bin/iroh-relay";
const ALPN: &[u8] = b"s11/echo";
/// One echo frame: an 8-byte sequence number and 1 KiB of payload.
const FRAME: usize = 8 + 1024;
const RESTART_CONFIG: &str = "config/local.toml";
const RESTART_ADDR: &str = "127.0.0.1:19101";
const RESTART_URL: &str = "http://127.0.0.1:19101";

/// Milliseconds from `t0` to `t`, to 0.1 ms.
fn ms_from(t0: Instant, t: Instant) -> f64 {
    (t.duration_since(t0).as_secs_f64() * 1e4).round() / 10.0
}

/// A stock `iroh-relay` process. Its stderr is appended to a log file.
struct Relay {
    child: Child,
}

impl Relay {
    fn start(config: &str, log: &Path) -> Result<Self> {
        let log = OpenOptions::new().create(true).append(true).open(log)?;
        let child = std::process::Command::new(RELAY_BIN)
            .args(["--config-path", config])
            .env("RUST_LOG", std::env::var("S11_RELAY_LOG").unwrap_or_else(|_| "info".into()))
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .context("spawn iroh-relay")?;
        Ok(Self { child })
    }

    fn signal(&self, sig: i32) {
        // SAFETY: kill(2) on the pid of a child this process spawned.
        unsafe { libc::kill(self.child.id() as i32, sig) };
    }

    /// Polls for exit. Returns the exit time and status.
    async fn wait_exit(&mut self) -> Result<(Instant, ExitStatus)> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok((Instant::now(), status));
            }
            sleep(Duration::from_micros(200)).await;
        }
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Polls until a TCP connect to `addr` succeeds. Returns when it did.
async fn wait_listening(addr: &str) -> Result<Instant> {
    let sock: SocketAddr = addr.parse()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect_timeout(&sock, Duration::from_millis(50)).is_ok() {
            return Ok(Instant::now());
        }
        if Instant::now() > deadline {
            bail!("{addr} not listening after 10 s");
        }
        sleep(Duration::from_millis(2)).await;
    }
}

/// An endpoint with no IP transport. Every packet goes through `relays`.
async fn bind_peer(relays: &[RelayUrl]) -> Result<Endpoint> {
    let map = relays
        .iter()
        .map(|url| RelayConfig::new(url.clone(), None))
        .collect();
    Ok(Endpoint::builder(Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Custom(map))
        .clear_ip_transports()
        .bind()
        .await?)
}

/// The URL of the connected home relay. Call only after `online()`.
fn connected_url(ep: &Endpoint) -> RelayUrl {
    let mut watch = ep.home_relay_status();
    let statuses = watch.get();
    statuses
        .iter()
        .find(|s| s.is_connected())
        .map(|s| s.url().clone())
        .expect("home relay connected")
}

/// Each change of "has a connected home relay", with the time it happened.
type HomeLog = Arc<Mutex<Vec<(Instant, bool)>>>;

fn watch_home(ep: &Endpoint) -> HomeLog {
    let log = HomeLog::default();
    let (ep, out) = (ep.clone(), log.clone());
    tokio::spawn(async move {
        let mut watch = ep.home_relay_status();
        let mut last = None;
        loop {
            let up = watch.get().iter().any(|s| s.is_connected());
            if last != Some(up) {
                last = Some(up);
                out.lock().unwrap().push((Instant::now(), up));
            }
            if watch.updated().await.is_err() {
                break;
            }
        }
    });
    log
}

/// The first time the home relay came back up after it was first seen down at or after `t0`.
fn home_back_up(log: &HomeLog, t0: Instant) -> Option<f64> {
    let log = log.lock().unwrap();
    let (down_at, _) = *log.iter().find(|(t, up)| *t >= t0 && !*up)?;
    let (up_at, _) = *log.iter().find(|(t, up)| *t > down_at && *up)?;
    Some(ms_from(t0, up_at))
}

/// Echo traffic for one scenario run, kept across reconnects.
#[derive(Default)]
struct Flow {
    next: AtomicU64,
    /// Frames to send with no pacing before going back to 100 per second.
    burst: AtomicU64,
    sent: Mutex<BTreeMap<u64, Instant>>,
    echoed: Mutex<BTreeMap<u64, Instant>>,
}

fn first_echo_after(flow: &Flow, t: Instant) -> Option<Instant> {
    flow.echoed.lock().unwrap().values().copied().filter(|&e| e > t).min()
}

/// Opens a bidirectional stream to `to` and runs a sender and a reader on it.
async fn open_flow(
    ep: &Endpoint,
    to: &EndpointAddr,
    flow: &Arc<Flow>,
    stop: &Arc<AtomicBool>,
) -> Result<Connection> {
    let conn = timeout(Duration::from_secs(20), ep.connect(to.clone(), ALPN))
        .await
        .context("connect timed out")??;
    let (send, recv) = conn.open_bi().await?;
    tokio::spawn(send_frames(send, flow.clone(), stop.clone()));
    tokio::spawn(read_echoes(recv, flow.clone()));
    Ok(conn)
}

async fn send_frames(mut send: SendStream, flow: Arc<Flow>, stop: Arc<AtomicBool>) {
    let mut tick = interval(Duration::from_millis(10));
    while !stop.load(Ordering::Relaxed) {
        if flow.burst.load(Ordering::Relaxed) > 0 {
            flow.burst.fetch_sub(1, Ordering::Relaxed);
        } else {
            tick.tick().await;
        }
        let seq = flow.next.fetch_add(1, Ordering::Relaxed);
        let mut frame = vec![0u8; FRAME];
        frame[..8].copy_from_slice(&seq.to_be_bytes());
        if send.write_all(&frame).await.is_err() {
            break;
        }
        flow.sent.lock().unwrap().insert(seq, Instant::now());
    }
}

async fn read_echoes(mut recv: RecvStream, flow: Arc<Flow>) {
    let mut buf = [0u8; FRAME];
    while recv.read_exact(&mut buf).await.is_ok() {
        let seq = u64::from_be_bytes(buf[..8].try_into().unwrap());
        flow.echoed.lock().unwrap().entry(seq).or_insert_with(Instant::now);
    }
}

/// The far side: echoes every frame back on each connection.
async fn serve_echo(ep: Endpoint) {
    while let Some(incoming) = ep.accept().await {
        tokio::spawn(async move {
            let Ok(accepting) = incoming.accept() else { return };
            let Ok(conn) = accepting.await else { return };
            let Ok((mut send, mut recv)) = conn.accept_bi().await else { return };
            let mut buf = [0u8; FRAME];
            while recv.read_exact(&mut buf).await.is_ok() {
                if send.write_all(&buf).await.is_err() {
                    break;
                }
            }
        });
    }
}

#[derive(Clone, Copy)]
struct Sig {
    name: &'static str,
    num: i32,
}

async fn restart_run(sig: Sig, down: Duration, run: u32) -> Result<Value> {
    let url: RelayUrl = RESTART_URL.parse()?;
    let log = PathBuf::from(format!(
        "logs/restart-{}-{}ms-run{run}-relay.log",
        sig.name,
        down.as_millis()
    ));
    let _ = fs::remove_file(&log);

    let mut relay = Relay::start(RESTART_CONFIG, &log)?;
    wait_listening(RESTART_ADDR).await?;
    let a = bind_peer(std::slice::from_ref(&url)).await?;
    let b = bind_peer(std::slice::from_ref(&url)).await?;
    timeout(Duration::from_secs(20), a.online()).await.context("A never online")?;
    timeout(Duration::from_secs(20), b.online()).await.context("B never online")?;
    let b_addr = b.addr();
    tokio::spawn(serve_echo(b.clone()));
    let (home_a, home_b) = (watch_home(&a), watch_home(&b));

    let flow = Arc::new(Flow::default());
    let stop = Arc::new(AtomicBool::new(false));
    let mut conn = open_flow(&a, &b_addr, &flow, &stop).await?;
    sleep(Duration::from_secs(3)).await;
    let warm_echoes = flow.echoed.lock().unwrap().len();

    // A 2 MiB unpaced burst, so the kill lands while frames are still queued on the relay path.
    flow.burst.store(2048, Ordering::Relaxed);
    sleep(Duration::from_millis(20)).await;
    let t_kill = Instant::now();
    let inflight: Vec<u64> = {
        let sent = flow.sent.lock().unwrap();
        let echoed = flow.echoed.lock().unwrap();
        sent.keys().filter(|s| !echoed.contains_key(s)).copied().collect()
    };
    relay.signal(sig.num);
    let (t_exit, status) = relay.wait_exit().await?;

    sleep(down).await;
    let _relay2 = Relay::start(RESTART_CONFIG, &log)?;
    let t_up = wait_listening(RESTART_ADDR).await?;

    // Watch the connection until an echo arrives after the relay died and the in-flight frames
    // have settled (or 10 s after the first echo). If the connection closed, re-dial it.
    let mut survived = true;
    let mut close_reason = None;
    let mut t_redial = None;
    let mut redialed = false;
    let mut redial_attempts = 0;
    let mut redial_error = None;
    let mut next_redial = Instant::now();
    let deadline = t_exit + Duration::from_secs(90);
    loop {
        if survived && let Some(reason) = conn.close_reason() {
            survived = false;
            info!(%reason, closed_after_relay_down_ms = ms_from(t_exit, Instant::now()), "connection closed");
            close_reason = Some(reason.to_string());
            t_redial = Some(Instant::now());
        }
        if !survived && !redialed && Instant::now() >= next_redial {
            redial_attempts += 1;
            match open_flow(&a, &b_addr, &flow, &stop).await {
                Ok(new_conn) => {
                    conn = new_conn;
                    redialed = true;
                    info!(attempt = redial_attempts, "re-dialled");
                }
                Err(e) => {
                    redial_error = Some(format!("{e:#}"));
                    info!(attempt = redial_attempts, error = %e, "re-dial failed");
                    next_redial = Instant::now() + Duration::from_secs(1);
                }
            }
        }
        if let Some(first) = first_echo_after(&flow, t_up) {
            let echoed = flow.echoed.lock().unwrap();
            let settled = inflight.iter().all(|s| echoed.contains_key(s));
            drop(echoed);
            if settled || first.elapsed() > Duration::from_secs(10) {
                break;
            }
        }
        if Instant::now() > deadline {
            break;
        }
        sleep(Duration::from_millis(10)).await;
    }

    let first_echo = first_echo_after(&flow, t_up);
    let redial_echo = t_redial.and_then(|t| first_echo_after(&flow, t));
    let lost = inflight
        .iter()
        .filter(|s| !flow.echoed.lock().unwrap().contains_key(s))
        .count();
    let exit = match status.signal() {
        Some(s) => format!("killed by signal {s}"),
        None => format!("exit {:?}", status.code()),
    };
    let row = json!({
        "signal": sig.name,
        "down_ms": down.as_millis() as u64,
        "run": run,
        "relay_exit": exit,
        "relay_exit_ms": ms_from(t_kill, t_exit),
        "relay_up_ms": ms_from(t_kill, t_up),
        "home_a_back_ms": home_back_up(&home_a, t_kill),
        "home_b_back_ms": home_back_up(&home_b, t_kill),
        "first_echo_after_up_ms": first_echo.map(|t| ms_from(t_kill, t)),
        "conn_survived": survived,
        "close_reason": close_reason,
        "redial_first_echo_ms": redial_echo.map(|e| ms_from(t_kill, e)),
        "redial_attempts": redial_attempts,
        "redial_error": redial_error,
        "inflight_at_kill": inflight.len(),
        "inflight_lost": lost,
        "warm_echoes": warm_echoes,
        "sent_total": flow.sent.lock().unwrap().len(),
        "echoed_total": flow.echoed.lock().unwrap().len(),
        "b_addr_relays": b_addr.relay_urls().count(),
        "b_addr_ips": b_addr.ip_addrs().count(),
    });

    stop.store(true, Ordering::Relaxed);
    conn.close(0u32.into(), b"s11 done");
    a.close().await;
    b.close().await;
    Ok(row)
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let n = v.len();
    Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
}

fn summarize(rows: &[Value]) -> Value {
    let keys = [
        "relay_exit_ms",
        "relay_up_ms",
        "home_a_back_ms",
        "home_b_back_ms",
        "first_echo_after_up_ms",
        "redial_first_echo_ms",
        "inflight_at_kill",
        "inflight_lost",
    ];
    let mut out = Map::new();
    for key in keys {
        let vals = rows.iter().filter_map(|r| r[key].as_f64()).collect();
        out.insert(key.into(), median(vals).map_or(Value::Null, |m| json!(m)));
    }
    out.insert("runs".into(), json!(rows.len()));
    out.insert(
        "conn_survived_runs".into(),
        json!(rows.iter().filter(|r| r["conn_survived"] == true).count()),
    );
    Value::Object(out)
}

async fn restart(args: &[String]) -> Result<()> {
    let sig = match args.first().map(String::as_str) {
        Some("TERM") => Sig { name: "SIGTERM", num: libc::SIGTERM },
        Some("KILL") => Sig { name: "SIGKILL", num: libc::SIGKILL },
        Some("INT") => Sig { name: "SIGINT", num: libc::SIGINT },
        _ => bail!("signal must be TERM, KILL or INT"),
    };
    let runs: u32 = args.get(1).context("runs")?.parse()?;
    let down = Duration::from_millis(args.get(2).context("down_ms")?.parse()?);
    fs::create_dir_all("logs")?;
    let mut rows = Vec::new();
    for run in 1..=runs {
        let row = restart_run(sig, down, run).await?;
        println!("{row}");
        rows.push(row);
    }
    let summary = summarize(&rows);
    println!("SUMMARY {summary}");
    let file = format!("logs/restart-{}-{}ms-summary.json", sig.name, down.as_millis());
    fs::write(file, serde_json::to_string_pretty(&json!({ "runs": rows, "median": summary }))?)?;
    Ok(())
}

/// Scenario 4. A is homed on one of two relays and reaches B, which is homed on the other.
async fn idle() -> Result<()> {
    let (u1, u2): (RelayUrl, RelayUrl) =
        ("http://127.0.0.1:19111".parse()?, "http://127.0.0.1:19112".parse()?);
    let _r1 = Relay::start("config/idle-r1.toml", Path::new("logs/idle-r1.log"))?;
    let _r2 = Relay::start("config/idle-r2.toml", Path::new("logs/idle-r2.log"))?;
    wait_listening("127.0.0.1:19111").await?;
    wait_listening("127.0.0.1:19112").await?;

    let a = bind_peer(&[u1.clone(), u2.clone()]).await?;
    timeout(Duration::from_secs(20), a.online()).await.context("A never online")?;
    let home_a = connected_url(&a);
    let non_home = if home_a == u1 { u2 } else { u1 };
    let b = bind_peer(std::slice::from_ref(&non_home)).await?;
    timeout(Duration::from_secs(20), b.online()).await.context("B never online")?;
    let b_addr = b.addr();
    tokio::spawn(serve_echo(b.clone()));
    let home_log = watch_home(&a);
    info!(%home_a, %non_home, "A is homed on the first relay and reaches B through the second");

    let conn = timeout(Duration::from_secs(20), a.connect(b_addr, ALPN)).await.context("connect")??;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&vec![7u8; FRAME]).await?;
    let mut buf = vec![0u8; FRAME];
    recv.read_exact(&mut buf).await?;
    info!("one frame echoed; no application traffic from here on");

    sleep(Duration::from_secs(70)).await;
    info!(open = conn.close_reason().is_none(), "70 s idle: QUIC still open (5 s keep-alives)");
    conn.close(0u32.into(), b"s11 idle test");
    info!("closed the QUIC connection; this is the last datagram A sends to the non-home relay");
    sleep(Duration::from_secs(150)).await;

    let transitions = home_log.lock().unwrap().clone();
    info!(?transitions, "home relay status changes over the whole run (time, connected)");
    drop((send, recv));
    Ok(())
}

/// A probe: bind a client with the given relay, and report whether it comes online.
async fn dial(args: &[String]) -> Result<()> {
    let url: RelayUrl = args.first().context("dial needs a relay URL")?.parse()?;
    let mut cas = Vec::new();
    let mut token = None;
    let mut timeout_s = 15;
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--ca" => cas.push(CertificateDer::from(fs::read(value)?)),
            "--token" => token = Some(value.clone()),
            "--timeout-s" => timeout_s = value.parse()?,
            other => bail!("unknown flag {other}"),
        }
    }

    let mut relay = RelayConfig::new(url.clone(), None);
    if let Some(t) = token {
        relay = relay.with_auth_token(t);
    }
    let mut builder = Endpoint::builder(Minimal)
        .relay_mode(RelayMode::Custom([relay].into_iter().collect()))
        .clear_ip_transports();
    if !cas.is_empty() {
        builder = builder.ca_tls_config(CaTlsConfig::custom_roots(cas));
    }
    let ep = builder.bind().await?;
    let t0 = Instant::now();
    match timeout(Duration::from_secs(timeout_s), ep.online()).await {
        Ok(()) => println!("ONLINE relay={url} after_ms={}", ms_from(t0, Instant::now())),
        Err(_) => {
            let mut watch = ep.home_relay_status();
            let statuses = watch.get();
            if statuses.is_empty() {
                println!("NOT_ONLINE relay={url} no_status_after_{timeout_s}s");
            }
            for s in statuses {
                println!(
                    "NOT_ONLINE relay={} connected={} last_error={:?} auth_denied={:?}",
                    s.url(),
                    s.is_connected(),
                    s.last_error().map(|e| e.to_string()),
                    s.auth_denied_reason(),
                );
            }
        }
    }
    ep.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("restart") => restart(&args[1..]).await,
        Some("idle") => idle().await,
        Some("dial") => dial(&args[1..]).await,
        _ => bail!(
            "usage: s11 restart <TERM|KILL|INT> <runs> <down_ms> | idle | \
             dial <relay_url> [--ca <der>]... [--token <t>] [--timeout-s <n>]"
        ),
    }
}
