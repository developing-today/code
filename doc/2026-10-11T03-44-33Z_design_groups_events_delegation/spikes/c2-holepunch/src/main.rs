//! C2 holepunch peers. Two processes, each in its own lab network namespace (see lab/run.sh):
//!
//!   c2-holepunch serve --port P --relay URL --ca FILE --ticket FILE
//!       Echo endpoint. Writes its relay-only ticket to FILE once it is online.
//!   c2-holepunch dial --port P --relay URL --ca FILE --ticket FILE [--window SECS]
//!       Dials the ticket, echoes every 250 ms for SECS, then prints a timeline and a summary.
//!
//! Both endpoints bind UDP port P, home on --relay, keep n0's relays in their maps, and run QAD
//! on every relay. Tickets carry relay URLs only (`AddressPolicy::RelayOnly`).

use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use c1_connect::connect::{AddressPolicy, n0_relays};
use iroh::endpoint::presets::Minimal;
use iroh::endpoint::{Connection, Incoming, PathEvent, RecvStream, SendStream};
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, RelayUrl, TransportAddr};
use iroh_relay::RelayQuicConfig;
use iroh_tickets::endpoint::EndpointTicket;
use n0_future::StreamExt;
use rustls_pki_types::CertificateDer;

const ALPN: &[u8] = b"c2/holepunch/0";
const ONLINE_LIMIT: Duration = Duration::from_secs(30);
const SAMPLE_EVERY: Duration = Duration::from_millis(250);
/// QAD port of the optional stand-in relay (see `--standin-url`).
const STANDIN_QAD_PORT: u16 = 7843;

struct Args {
    command: String,
    port: u16,
    relay: RelayUrl,
    ca: PathBuf,
    ticket: PathBuf,
    window: u64,
    /// Whether the home relay runs QAD (the client asks it for its public address).
    home_qad: bool,
    /// Whether n0's relays are in the map. They are, unless a control run removes them.
    n0: bool,
    /// A second relay in the map that runs QAD but serves no relay traffic (`enable_relay = false`).
    standin: Option<RelayUrl>,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut it = std::env::args().skip(1);
        let command = it
            .next()
            .context("usage: c2-holepunch serve|dial --port P --relay URL --ca FILE --ticket FILE [--window SECS] [--home-qad on|off] [--n0 on|off]")?;
        let (mut port, mut relay, mut ca, mut ticket) = (None, None, None, None);
        let (mut window, mut home_qad, mut n0) = (60, true, true);
        let mut standin = None;
        while let Some(flag) = it.next() {
            let value = it.next().with_context(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--port" => port = Some(value.parse()?),
                "--relay" => relay = Some(value.parse()?),
                "--ca" => ca = Some(value.into()),
                "--ticket" => ticket = Some(value.into()),
                "--window" => window = value.parse()?,
                "--home-qad" => home_qad = on_off(&value)?,
                "--n0" => n0 = on_off(&value)?,
                "--standin-url" => standin = Some(value.parse()?),
                other => bail!("unknown flag {other}"),
            }
        }
        Ok(Self {
            command,
            port: port.context("--port")?,
            relay: relay.context("--relay")?,
            ca: ca.context("--ca")?,
            ticket: ticket.context("--ticket")?,
            window,
            home_qad,
            n0,
            standin,
        })
    }
}

fn on_off(value: &str) -> Result<bool> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        other => bail!("expected on or off, got {other}"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // iroh's events go to stderr, filtered by RUST_LOG (e.g. iroh::_events::qnt=debug). Unset, only errors.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let args = Args::parse()?;
    match args.command.as_str() {
        "serve" => serve(&args).await,
        "dial" => dial(&args).await,
        other => bail!("unknown command {other}"),
    }
}

/// An endpoint on UDP `args.port`, homed on `args.relay`. The map also holds n0's relays, each
/// with QAD on (`RelayConfig::from`). The home relay has QAD unless `--home-qad off`. No lookup
/// service is configured.
async fn bind(args: &Args, alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    let home = if args.home_qad {
        RelayConfig::from(args.relay.clone())
    } else {
        RelayConfig::new(args.relay.clone(), None)
    };
    let mut configs = vec![home];
    if args.n0 {
        configs.extend(n0_relays().into_iter().map(RelayConfig::from));
    }
    if let Some(url) = &args.standin {
        configs.push(RelayConfig::new(url.clone(), Some(RelayQuicConfig::new(STANDIN_QAD_PORT))));
    }
    let relays: RelayMap = configs.into_iter().collect();
    // The lab's relay certificate is issued by a lab CA. Trust that CA only.
    let ca = CertificateDer::from(fs::read(&args.ca).context("reading --ca")?);
    let endpoint = Endpoint::builder(Minimal)
        .alpns(alpns)
        .relay_mode(RelayMode::Custom(relays))
        .addr_filter(AddressPolicy::RelayOnly.filter())
        .ca_tls_config(CaTlsConfig::custom_roots([ca]))
        .clear_ip_transports()
        .bind_addr(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, args.port))?
        .bind()
        .await?;
    Ok(endpoint)
}

/// Waits for a home relay, then prints when it was reached and which relay is home.
async fn wait_online(endpoint: &Endpoint, origin: Instant) -> Result<()> {
    tokio::time::timeout(ONLINE_LIMIT, endpoint.online())
        .await
        .context("not online within 30 s")?;
    let home: Vec<String> = endpoint.addr().relay_urls().map(|u| u.to_string()).collect();
    println!("{:>7} ms online home_relays={home:?}", ms(origin));
    Ok(())
}

async fn serve(args: &Args) -> Result<()> {
    let started = Instant::now();
    let endpoint = bind(args, vec![ALPN.to_vec()]).await?;
    wait_online(&endpoint, started).await?;
    // QAD can finish a moment after the home relay connects. Give it time to learn the public address.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let own: Vec<String> = endpoint.addr().ip_addrs().map(|a| a.to_string()).collect();
    println!("{:>7} ms own_ip_candidates={own:?}", ms(started));
    let ticket = EndpointTicket::new(AddressPolicy::RelayOnly.apply(&endpoint.addr()));
    fs::write(&args.ticket, ticket.to_string()).context("writing --ticket")?;
    println!("ticket written: {} chars", ticket.to_string().len());
    while let Some(incoming) = endpoint.accept().await {
        tokio::spawn(answer(incoming, started));
    }
    Ok(())
}

/// Echoes every stream on one connection, and logs that connection's path changes.
async fn answer(incoming: Incoming, origin: Instant) -> Result<()> {
    let conn = incoming.await?;
    log_paths(&conn, "serve", origin, Arc::default());
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let request = read_frame(&mut recv).await?;
        let mut reply = b"echo: ".to_vec();
        reply.extend_from_slice(&request);
        write_frame(&mut send, &reply).await?;
        send.finish()?;
    }
    Ok(())
}

/// One path change, for the summary: when, and what was selected.
type Selection = (u128, String);

async fn dial(args: &Args) -> Result<()> {
    let started = Instant::now();
    println!("profile home_qad={} n0={} standin={:?}", args.home_qad, args.n0, args.standin);
    let endpoint = bind(args, vec![]).await?;
    wait_online(&endpoint, started).await?;

    let text = fs::read_to_string(&args.ticket).context("reading --ticket")?;
    let ticket: EndpointTicket = text.trim().parse()?;
    let addr = ticket.endpoint_addr().clone();
    ensure!(addr.ip_addrs().count() == 0, "ticket carries a direct address");
    let server_relays: Vec<String> = addr.relay_urls().map(|u| u.to_string()).collect();
    println!("ticket relays={server_relays:?} direct=0");

    // Everything below is measured from the dial start, `t0`.
    let t0 = Instant::now();
    let selections: Arc<Mutex<Vec<Selection>>> = Arc::default();
    let conn = endpoint.connect(addr, ALPN).await.context("connect")?;
    let connected_ms = ms(t0);
    let handshake_path = selected(&conn);
    println!("{connected_ms:>7} ms handshake done, selected={handshake_path}");
    log_paths(&conn, "dial", t0, selections.clone());

    let (reply, _) = echo(&conn).await?;
    let first_data_ms = ms(t0);
    let first_data_path = selected(&conn);
    println!("{first_data_ms:>7} ms first reply ({} bytes), selected={first_data_path}", reply.len());

    // Hold the connection for the window and echo, to see whether a direct path holds.
    let mut rtts: Vec<(f64, bool)> = Vec::new(); // (rtt ms, selected path was direct)
    let mut failures = 0;
    let window_end = Instant::now() + Duration::from_secs(args.window);
    while Instant::now() < window_end && conn.close_reason().is_none() {
        let sent = Instant::now();
        match echo(&conn).await {
            Ok(_) => rtts.push((sent.elapsed().as_secs_f64() * 1e3, selected(&conn).starts_with("direct"))),
            Err(err) => {
                failures += 1;
                println!("{:>7} ms echo failed: {err:#}", ms(t0));
            }
        }
        tokio::time::sleep(SAMPLE_EVERY).await;
    }

    let selected_at_end = selected(&conn);
    let own: Vec<String> = endpoint.addr().ip_addrs().map(|a| a.to_string()).collect();
    let attempts = endpoint.metrics().socket.holepunch_attempts.get();
    let selections = selections.lock().unwrap().clone();
    let first_direct = selections.iter().find(|(_, kind)| kind.starts_with("direct"));

    println!("--- summary (dial)");
    println!("connected_ms={connected_ms} handshake_path={handshake_path}");
    println!("first_data_ms={first_data_ms} first_data_path={first_data_path}");
    match first_direct {
        Some((at, kind)) => println!("first_direct_selected_ms={at} ({kind})"),
        None => println!("first_direct_selected_ms=none"),
    }
    println!("selections={selections:?}");
    let on_direct = rtts.iter().filter(|(_, d)| *d).count();
    println!(
        "window_s={} echoes_ok={} echoes_failed={failures} echoes_on_direct={on_direct} echoes_on_relay={}",
        args.window,
        rtts.len(),
        rtts.len() - on_direct
    );
    println!("rtt_ms_median_relay={:?}", median(rtts.iter().filter(|(_, d)| !*d).map(|(r, _)| *r)));
    println!("rtt_ms_median_direct={:?}", median(rtts.iter().filter(|(_, d)| *d).map(|(r, _)| *r)));
    println!("selected_at_end={selected_at_end}");
    println!("holepunch_attempts={attempts}");
    println!("own_ip_candidates={own:?}");

    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok(())
}

/// Prints each path change on `conn`, in ms from `origin`, and records the selections.
fn log_paths(conn: &Connection, who: &'static str, origin: Instant, log: Arc<Mutex<Vec<Selection>>>) {
    let mut events = conn.path_events();
    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            let at = origin.elapsed().as_millis();
            match event {
                PathEvent::Opened { remote_addr, .. } => {
                    println!("{at:>7} ms {who} path opened: {}", kind(&remote_addr));
                }
                PathEvent::Selected { remote_addr, .. } => {
                    let text = kind(&remote_addr);
                    println!("{at:>7} ms {who} path selected: {text}");
                    log.lock().unwrap().push((at, text));
                }
                PathEvent::Closed { remote_addr, .. } => {
                    println!("{at:>7} ms {who} path closed: {}", kind(&remote_addr));
                }
                PathEvent::Lagged { missed, .. } => println!("{at:>7} ms {who} events lagged: {missed}"),
                _ => {}
            }
        }
    });
}

/// The path that carries application data now: "relay <url>" or "direct <ip:port>", or "none".
fn selected(conn: &Connection) -> String {
    conn.paths()
        .iter()
        .find(|p| p.is_selected())
        .map(|p| kind(p.remote_addr()))
        .unwrap_or_else(|| "none".into())
}

fn kind(addr: &TransportAddr) -> String {
    match addr {
        TransportAddr::Relay(url) => format!("relay {url}"),
        TransportAddr::Ip(ip) => format!("direct {ip}"),
        _ => "other".into(),
    }
}

/// One request and reply on a new stream. Returns the reply and the time it took.
async fn echo(conn: &Connection) -> Result<(Vec<u8>, Duration)> {
    let started = Instant::now();
    let (mut send, mut recv) = conn.open_bi().await?;
    write_frame(&mut send, b"hello from the dialer").await?;
    send.finish()?;
    let reply = read_frame(&mut recv).await?;
    Ok((reply, started.elapsed()))
}

async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<()> {
    send.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    send.write_all(payload).await?;
    Ok(())
}

async fn read_frame(recv: &mut RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
    recv.read_exact(&mut buf).await?;
    Ok(buf)
}

fn ms(origin: Instant) -> u128 {
    origin.elapsed().as_millis()
}

fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut v: Vec<f64> = values.collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(((v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0 * 10.0).round() / 10.0)
}
