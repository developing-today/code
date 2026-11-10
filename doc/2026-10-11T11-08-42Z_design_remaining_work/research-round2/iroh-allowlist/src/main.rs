//! N30 spike: refusing an unlisted EndpointId in iroh 1.3.0.
//!
//! Two roles, run as two processes in one network namespace (lab/run.sh), so each side logs only
//! what it saw. Logs go to stderr with wall-clock timestamps; lab/summarize.py merges them.
//!
//!   iroh-allowlist-spike server --bind IP:0 --mode refuse|app|filter --ready FILE
//!   iroh-allowlist-spike client --seed N --bind IP:0 --kind connect|stream --ticket FILE
//!
//! Server modes:
//!   refuse  EndpointHooks::after_handshake rejects ids that are not on the allowlist. This is the
//!           only hook that sees the peer's EndpointId.
//!   app     after_handshake accepts everyone. The first bi stream is checked against the list.
//!   filter  Incoming::refuse() before any handshake, for everyone. No identity is available here.
//!
//! Fixed seeds, so the allowlist is explicit: server 1, allowed client 2, unlisted client 3.

use std::{
    collections::HashSet,
    future::Future,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey,
    endpoint::{AfterHandshakeOutcome, Connection, EndpointHooks, VarInt, presets},
};
use tracing::info;
use tracing_subscriber::EnvFilter;

const ALPN: &[u8] = b"n30/allowlist/0";
const SERVER_SEED: u8 = 1;
const ALLOWED_SEED: u8 = 2;
// Codes sent in CONNECTION_CLOSE, so the refused side can name the refusal.
const HOOK_CLOSE: u32 = 0x1001;
const APP_CLOSE: u32 = 0x1002;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Refuse,
    App,
    Filter,
}

fn seed_key(seed: u8) -> SecretKey {
    SecretKey::from_bytes(&[seed; 32])
}

fn arg(args: &[String], name: &str) -> Result<String> {
    let flag = format!("--{name}");
    let i = args
        .iter()
        .position(|a| a == &flag)
        .with_context(|| format!("missing {flag}"))?;
    args.get(i + 1)
        .cloned()
        .with_context(|| format!("{flag} needs a value"))
}

/// Addresses the endpoint holds for `id` right now, with their usage.
async fn snapshot(ep: &Endpoint, id: EndpointId) -> Vec<String> {
    match ep.remote_info(id).await {
        Some(info) => info
            .addrs()
            .map(|a| format!("{:?}/{:?}", a.addr(), a.usage()))
            .collect(),
        None => vec!["<no remote info>".to_string()],
    }
}

/// Runs in `EndpointHooks::after_handshake`. The TLS handshake is done, so the peer's id is known.
#[derive(Debug)]
struct Gate {
    allowed: Arc<HashSet<EndpointId>>,
    enforce: bool,
    last_peer: Arc<Mutex<Option<EndpointId>>>,
}

impl EndpointHooks for Gate {
    fn after_handshake<'a>(
        &'a self,
        conn: &'a Connection,
    ) -> impl Future<Output = AfterHandshakeOutcome> + Send + 'a {
        async move {
            let peer = conn.remote_id();
            *self.last_peer.lock().unwrap() = Some(peer);
            let allowed = self.allowed.contains(&peer);
            info!(target: "harness", role = "server", event = "after_handshake", peer = %peer.fmt_short(), allowed, enforce = self.enforce, "hook runs");
            if self.enforce && !allowed {
                AfterHandshakeOutcome::Reject {
                    error_code: VarInt::from_u32(HOOK_CLOSE),
                    reason: b"not on allowlist".to_vec(),
                }
            } else {
                AfterHandshakeOutcome::Accept
            }
        }
    }
}

async fn server(args: &[String]) -> Result<()> {
    let bind = arg(args, "bind")?;
    let mode = match arg(args, "mode")?.as_str() {
        "refuse" => Mode::Refuse,
        "app" => Mode::App,
        "filter" => Mode::Filter,
        other => bail!("unknown --mode {other}"),
    };
    let ready = PathBuf::from(arg(args, "ready")?);
    let allowed: Arc<HashSet<EndpointId>> =
        Arc::new([seed_key(ALLOWED_SEED).public()].into_iter().collect());
    let last_peer = Arc::new(Mutex::new(None));

    let server = Endpoint::builder(presets::Minimal)
        .secret_key(seed_key(SERVER_SEED))
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(bind.as_str())?
        .alpns(vec![ALPN.to_vec()])
        .hooks(Gate {
            allowed: allowed.clone(),
            enforce: mode == Mode::Refuse,
            last_peer: last_peer.clone(),
        })
        .bind()
        .await?;

    // Give the interface scan time to finish, so the candidate set is complete before a peer dials.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let port = server
        .bound_sockets()
        .first()
        .context("no bound socket")?
        .port();
    let direct: Vec<String> = server.addr().ip_addrs().map(|a| a.to_string()).collect();
    info!(target: "harness", role = "server", event = "ready", mode = ?mode, id = %server.id().fmt_short(), port, direct = ?direct, "bound");
    let tmp = ready.with_extension("tmp");
    std::fs::write(&tmp, format!("id={}\nport={}\n", server.id(), port))?;
    std::fs::rename(&tmp, &ready)?;

    loop {
        let Some(incoming) = server.accept().await else {
            break;
        };
        // ALPNs from the ClientHello in the Initial packet: the only peer data readable before the handshake.
        let alpns: Vec<String> = incoming
            .decrypt()
            .and_then(|d| d.alpns())
            .map(|it| {
                it.map(|r| match r {
                    Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                    Err(_) => "<truncated>".to_string(),
                })
                .collect()
            })
            .unwrap_or_default();
        info!(target: "harness", role = "server", event = "incoming", remote = ?incoming.remote_addr(), alpns = ?alpns, "first Initial packet; no EndpointId is known yet");
        if mode == Mode::Filter {
            info!(target: "harness", role = "server", event = "refused_before_handshake", "Incoming::refuse()");
            incoming.refuse();
            continue;
        }
        let accepting = match incoming.accept() {
            Ok(accepting) => accepting,
            Err(err) => {
                info!(target: "harness", role = "server", event = "accept_error", error = %err, "Incoming::accept failed");
                continue;
            }
        };
        let ep = server.clone();
        let allowed = allowed.clone();
        let last_peer = last_peer.clone();
        tokio::spawn(async move {
            match accepting.await {
                Ok(conn) => serve(conn, &ep, mode, &allowed).await,
                Err(err) => {
                    let peer = *last_peer.lock().unwrap();
                    let known = match peer {
                        Some(p) => snapshot(&ep, p).await,
                        None => vec!["<no peer id>".to_string()],
                    };
                    info!(target: "harness", role = "server", event = "handshake_refused", peer = ?peer.map(|p| p.fmt_short().to_string()), error = %err, dbg = ?err, remote_info_after_refusal = ?known, "no Connection was handed to the application");
                }
            }
        });
    }
    Ok(())
}

/// The application side of a connection that the hook accepted.
async fn serve(conn: Connection, ep: &Endpoint, mode: Mode, allowed: &HashSet<EndpointId>) {
    let peer = conn.remote_id();
    let ok = allowed.contains(&peer);
    let at_handoff = snapshot(ep, peer).await;
    info!(target: "harness", role = "server", event = "application_has_connection", peer = %peer.fmt_short(), allowed = ok, remote_info = ?at_handoff, "Connection handed to the application");
    match tokio::time::timeout(Duration::from_secs(10), conn.accept_bi()).await {
        Ok(Ok((mut send, mut recv))) => {
            let line = recv.read_to_end(256).await.unwrap_or_default();
            let at_stream = snapshot(ep, peer).await;
            info!(target: "harness", role = "server", event = "stream_opened", peer = %peer.fmt_short(), first_bytes = %String::from_utf8_lossy(&line).trim_end(), remote_info = ?at_stream, "stream from peer");
            if ok {
                let _ = send.write_all(b"welcome\n").await;
                let _ = send.finish();
            } else if mode == Mode::App {
                let at_refusal = snapshot(ep, peer).await;
                info!(target: "harness", role = "server", event = "app_refusal", peer = %peer.fmt_short(), remote_info = ?at_refusal, "refusing after the stream opened");
                conn.close(VarInt::from_u32(APP_CLOSE), b"not on allowlist");
            }
        }
        Ok(Err(err)) => {
            info!(target: "harness", role = "server", event = "accept_bi_error", error = %err, "no stream")
        }
        Err(_) => info!(target: "harness", role = "server", event = "accept_bi_timeout", "no stream within 10 s"),
    }
    let why = conn.closed().await;
    info!(target: "harness", role = "server", event = "connection_closed", why = %why, "closed");
}

fn read_ready(path: &Path) -> Result<(EndpointId, u16)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let field = |key: &str| -> Result<String> {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .map(str::to_string)
            .with_context(|| format!("no {key}= in {}", path.display()))
    };
    Ok((field("id")?.parse()?, field("port")?.parse()?))
}

async fn client(args: &[String]) -> Result<()> {
    let seed: u8 = arg(args, "seed")?.parse()?;
    let bind = arg(args, "bind")?;
    let kind = arg(args, "kind")?;
    let (server_id, port) = read_ready(Path::new(&arg(args, "ticket")?))?;

    let ep = Endpoint::builder(presets::Minimal)
        .secret_key(seed_key(seed))
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(bind.as_str())?
        .bind()
        .await?;
    // The ticket: the server's EndpointId and one loopback address. Nothing else.
    let ticket =
        EndpointAddr::new(server_id).with_ip_addr(SocketAddr::from(([127, 0, 0, 1], port)));
    info!(target: "harness", role = "client", event = "start", me = %ep.id().fmt_short(), kind = %kind, ticket = ?ticket.ip_addrs().collect::<Vec<_>>(), "dialing with one loopback hint");

    let t0 = Instant::now();
    let conn = match ep.connect(ticket, ALPN).await {
        Ok(conn) => {
            let known = snapshot(&ep, server_id).await;
            info!(target: "harness", role = "client", event = "connect_ok", elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0, remote_info = ?known, "connect() returned a Connection");
            conn
        }
        Err(err) => {
            info!(target: "harness", role = "client", event = "connect_err", elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0, error = %err, dbg = ?err, "connect() failed");
            ep.close().await;
            return Ok(());
        }
    };

    if kind == "stream" {
        match conn.open_bi().await {
            Ok((mut send, mut recv)) => {
                let _ = send.write_all(b"hello\n").await;
                let _ = send.finish();
                match recv.read_to_end(256).await {
                    Ok(bytes) => {
                        info!(target: "harness", role = "client", event = "stream_reply", elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0, reply = %String::from_utf8_lossy(&bytes).trim_end(), "reply")
                    }
                    Err(err) => {
                        info!(target: "harness", role = "client", event = "stream_error", elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0, error = %err, dbg = ?err, "reading reply failed")
                    }
                }
            }
            Err(err) => {
                info!(target: "harness", role = "client", event = "open_bi_error", elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0, error = %err, dbg = ?err, "open_bi failed")
            }
        }
    }

    // Wait for the server to close. If it does not within 3 s, close from this side.
    match tokio::time::timeout(Duration::from_secs(3), conn.closed()).await {
        Ok(why) => {
            let known = snapshot(&ep, server_id).await;
            info!(target: "harness", role = "client", event = "closed_by_peer", elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0, why = %why, dbg = ?why, remote_info = ?known, "peer closed")
        }
        Err(_) => {
            conn.close(VarInt::from_u32(0), b"client done");
            info!(target: "harness", role = "client", event = "closed_locally", elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0, "no close from peer in 3 s; closed here")
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let final_info = snapshot(&ep, server_id).await;
    info!(target: "harness", role = "client", event = "remote_info_final", remote_info = ?final_info, "final snapshot");
    ep.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "harness=info,iroh::socket::remote_map::remote_state=trace,iroh=warn,noq_proto=warn",
        )
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("server") => server(&args[2..]).await,
        Some("client") => client(&args[2..]).await,
        _ => bail!("usage: iroh-allowlist-spike server|client ..."),
    }
}
