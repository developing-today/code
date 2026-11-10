//! Helpers for the s7 tests: a local relay, a one-request echo protocol, the mainline testnet,
//! and a read-back of what a stored DHT record contains.

use std::fs::File;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iroh::endpoint::{Connection, Endpoint, Incoming, RecvStream, SendStream};
use iroh::endpoint_info::EndpointInfo;
use iroh::{EndpointAddr, EndpointId, RelayUrl, TransportAddr};
use iroh_dns::pkarr::{SignedPacket, Timestamp};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use n0_future::StreamExt;
use n0_mainline::{Dht, Testnet};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;

/// Reply prefix. The dialer checks for `echo: hello from the dialer`.
pub const ECHO_PREFIX: &[u8] = b"echo: ";
pub const REQUEST: &[u8] = b"hello from the dialer";

/// Starts an `iroh-relay` in HTTP mode on loopback.
///
/// The URL uses the name `localhost`, so the relay's address is not an IP literal in a
/// published record. The server listens on 127.0.0.1 only.
pub async fn local_relay() -> Result<(Server, RelayUrl)> {
    let mut config = ServerConfig::default();
    config.relay = Some(RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
    let server = Server::spawn(config).await?;
    let port = server
        .http_addr()
        .context("relay has no HTTP address")?
        .port();
    let url: RelayUrl = format!("http://localhost:{port}").parse()?;
    Ok((server, url))
}

async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<()> {
    send.write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
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

/// Echoes each request as `echo: <request>`, one task per connection.
pub fn serve(endpoint: Endpoint) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            tokio::spawn(async move {
                let _ = answer(incoming).await;
            });
        }
    })
}

async fn answer(incoming: Incoming) -> Result<()> {
    let conn = incoming.await?;
    let (mut send, mut recv) = conn.accept_bi().await?;
    let request = read_frame(&mut recv).await?;
    let mut reply = ECHO_PREFIX.to_vec();
    reply.extend_from_slice(&request);
    write_frame(&mut send, &reply).await?;
    send.finish()?;
    conn.closed().await;
    Ok(())
}

/// One request and reply on a fresh connection. The connection stays open so its paths can
/// be read.
pub struct Dialed {
    pub reply: Vec<u8>,
    /// Time until the connection was established.
    pub connected: Duration,
    /// Time until the reply arrived.
    pub replied: Duration,
    /// The relay the connection was established over, read right after the handshake. `None`
    /// means a direct path. Later holepunching can add a direct path, so this is not read after
    /// the reply.
    pub handshake_relay: Option<RelayUrl>,
    pub conn: Connection,
}

pub fn echoed(d: &Dialed) -> bool {
    d.reply.strip_prefix(ECHO_PREFIX) == Some(REQUEST)
}

/// Dials `addr` once.
pub async fn dial(client: &Endpoint, addr: impl Into<EndpointAddr>, alpn: &[u8]) -> Result<Dialed> {
    let started = Instant::now();
    let conn = client.connect(addr, alpn).await.context("connect")?;
    let connected = started.elapsed();
    let handshake_relay = selected_relay(&conn);
    let (mut send, mut recv) = conn.open_bi().await?;
    write_frame(&mut send, REQUEST).await?;
    send.finish()?;
    let reply = read_frame(&mut recv).await?;
    Ok(Dialed {
        reply,
        connected,
        replied: started.elapsed(),
        handshake_relay,
        conn,
    })
}

/// The relay URLs in `endpoint`'s current address.
pub fn home_relays(endpoint: &Endpoint) -> Vec<RelayUrl> {
    endpoint.addr().relay_urls().cloned().collect()
}

/// Result of `dial_by_id`.
pub struct ByIdDial {
    pub dialed: Dialed,
    /// Attempts made, including the successful one.
    pub attempts: u32,
    /// Error text from each failed attempt.
    pub failures: Vec<String>,
    /// Time from the first attempt to the reply.
    pub since_first: Duration,
}

/// Dials `id` with no ticket, retrying every 200 ms until `limit`. Each attempt runs a fresh
/// lookup, so a failure can be a DHT node that is not bootstrapped yet.
pub async fn dial_by_id(
    client: &Endpoint,
    id: EndpointId,
    alpn: &[u8],
    limit: Duration,
) -> Result<ByIdDial> {
    let started = Instant::now();
    let mut attempts = 0;
    let mut failures = Vec::new();
    loop {
        attempts += 1;
        match dial(client, id, alpn).await {
            Ok(dialed) => {
                return Ok(ByIdDial {
                    dialed,
                    attempts,
                    failures,
                    since_first: started.elapsed(),
                });
            }
            Err(err) => {
                failures.push(format!("{err:#}"));
                if started.elapsed() >= limit {
                    bail!("no connection after {attempts} attempts: {failures:?}");
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

/// The paths on `conn`, with `*` on the selected one. IP addresses are not printed.
pub fn paths(conn: &Connection) -> Vec<String> {
    conn.paths()
        .iter()
        .map(|p| {
            let mark = if p.is_selected() { "*" } else { " " };
            let kind = match p.remote_addr() {
                TransportAddr::Relay(url) => format!("relay {url}"),
                _ => "direct (address withheld)".to_string(),
            };
            format!("{mark} {kind}")
        })
        .collect()
}

/// The relay URL of the selected path, or `None` if the selected path is direct.
pub fn selected_relay(conn: &Connection) -> Option<RelayUrl> {
    conn.paths().iter().find_map(|p| match p.remote_addr() {
        TransportAddr::Relay(url) if p.is_selected() => Some(url.clone()),
        _ => None,
    })
}

/// Waits until `endpoint` is online (connected to a home relay), and returns how long that took.
pub async fn online(endpoint: &Endpoint, limit: Duration) -> Result<Duration> {
    let started = Instant::now();
    tokio::time::timeout(limit, endpoint.online())
        .await
        .context("never came online")?;
    Ok(started.elapsed())
}

/// Items that each lookup service returns for `id`, collected for up to `limit`. Shows which
/// service knows which address, before any dial.
pub async fn lookup_items(
    endpoint: &Endpoint,
    id: EndpointId,
    limit: Duration,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stream = Box::pin(endpoint.address_lookup()?.resolve(id));
    let _ = tokio::time::timeout(limit, async {
        while let Some(item) = stream.next().await {
            out.push(match item {
                Ok(Ok(item)) => format!(
                    "{}: {} relay, {} direct",
                    item.provenance(),
                    item.relay_urls().count(),
                    item.ip_addrs().count()
                ),
                Ok(Err(err)) => format!("error: {err:?}"),
                Err(err) => format!("no result: {err:?}"),
            });
        }
    })
    .await;
    Ok(out)
}

/// A mainline testnet of `nodes` DHT nodes on loopback. Refuses any bootstrap node that is
/// not on loopback, so the public DHT cannot be reached through it.
pub async fn testnet(nodes: usize) -> Result<Testnet> {
    let net = Testnet::new(nodes).await?;
    if net.bootstrap.iter().any(|b| !b.starts_with("127.0.0.1:")) {
        bail!("testnet bootstrap is not on loopback");
    }
    Ok(net)
}

/// A DHT client that bootstraps on the testnet only. Used to read records back.
pub fn testnet_dht(net: &Testnet) -> Result<Dht> {
    let mut builder = Dht::builder();
    builder.bootstrap(&net.bootstrap);
    Ok(builder.build()?)
}

/// The record stored under `id` on the DHT, read back as a signed packet.
pub async fn fetch_record(dht: &Dht, id: EndpointId) -> Result<Option<SignedPacket>> {
    let Some(item) = dht.get_mutable_most_recent(id.as_bytes(), None).await? else {
        return Ok(None);
    };
    let packet = SignedPacket::from_parts_unchecked(
        item.key(),
        item.signature(),
        Timestamp::from_micros(item.seq() as u64),
        item.value(),
    )?;
    Ok(Some(packet))
}

/// What a stored record says, read two ways: as parsed endpoint info, and as raw bytes.
pub struct Audit {
    /// Runs of printable text in the record, which hold the attributes such as `relay=…`.
    pub text: Vec<String>,
    pub relay_urls: Vec<String>,
    /// Direct addresses after parsing.
    pub parsed_ip_addrs: usize,
    /// IP literals found anywhere in the record's bytes.
    pub ip_literals: Vec<String>,
}

pub fn audit(packet: &SignedPacket) -> Result<Audit> {
    let info = EndpointInfo::from_pkarr_signed_packet(packet)?;
    let raw: &[u8] = packet.encoded_packet();
    Ok(Audit {
        text: printable_runs(raw, 4),
        relay_urls: info.relay_urls().map(|u| u.to_string()).collect(),
        parsed_ip_addrs: info.ip_addrs().count(),
        ip_literals: ip_literals(&String::from_utf8_lossy(raw)),
    })
}

/// Runs of printable ASCII at least `min` bytes long.
fn printable_runs(bytes: &[u8], min: usize) -> Vec<String> {
    bytes
        .split(|b| !(0x20..0x7f).contains(b))
        .filter(|run| run.len() >= min)
        .map(|run| String::from_utf8_lossy(run).into_owned())
        .collect()
}

/// IPv4 and IPv6 literals in `text`.
pub fn ip_literals(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for token in text.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        if token.parse::<Ipv4Addr>().is_ok() {
            found.push(token.to_string());
        }
    }
    for token in text.split(|c: char| !(c.is_ascii_hexdigit() || c == ':')) {
        if token.matches(':').count() >= 2 && token.parse::<Ipv6Addr>().is_ok() {
            found.push(token.to_string());
        }
    }
    found
}

/// Polls the testnet until `id`'s record appears, or fails after `limit`.
pub async fn wait_for_record(
    dht: &Dht,
    id: EndpointId,
    limit: Duration,
) -> Result<(SignedPacket, Duration)> {
    let started = Instant::now();
    loop {
        if let Some(packet) = fetch_record(dht, id).await? {
            return Ok((packet, started.elapsed()));
        }
        if started.elapsed() >= limit {
            bail!("record not on the testnet after {limit:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// An `iroh-relay` child process. Killed on drop, so a failed test does not leave the port held.
pub struct ExternalRelay {
    child: Child,
}

impl Drop for ExternalRelay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `bin --config-path <config>` with `RUST_LOG=info`, its output going to `log`. Returns
/// once `GET <base>/ping` answers 200, or fails after 10 s.
pub async fn external_relay(
    bin: &Path,
    config: &Path,
    log: &Path,
    base: &str,
) -> Result<ExternalRelay> {
    let out = File::create(log)?;
    let err = out.try_clone()?;
    let child = Command::new(bin)
        .arg("--config-path")
        .arg(config)
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .with_context(|| format!("starting {}", bin.display()))?;
    let mut relay = ExternalRelay { child };
    let started = Instant::now();
    loop {
        if let Some(status) = relay.child.try_wait()? {
            bail!(
                "relay exited before /ping answered ({status}); see {}",
                log.display()
            );
        }
        if http_status(base, "/ping").await.ok() == Some(200) {
            return Ok(relay);
        }
        if started.elapsed() >= Duration::from_secs(10) {
            bail!(
                "relay /ping did not return 200 within 10 s; see {}",
                log.display()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The status code of a plain HTTP/1.1 GET of `path` on `base` (`http://host:port`). The request
/// is written by hand, so the probe needs no HTTP client crate.
pub async fn http_status(base: &str, path: &str) -> Result<u16> {
    let addr = base
        .strip_prefix("http://")
        .context("only http:// bases are supported")?;
    let limit = Duration::from_secs(2);
    let mut stream = tokio::time::timeout(limit, TcpStream::connect(addr)).await??;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut reply = Vec::new();
    tokio::time::timeout(limit, stream.read_to_end(&mut reply)).await??;
    let text = String::from_utf8_lossy(&reply);
    let line = text.lines().next().context("empty reply")?;
    let code = line.split_whitespace().nth(1).context("no status code")?;
    Ok(code.parse()?)
}
