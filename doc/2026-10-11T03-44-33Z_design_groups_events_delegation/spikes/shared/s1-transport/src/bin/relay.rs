//! Spike S1 part (f): a relay-only connection through a local iroh-relay in HTTP mode.
//!
//! Both endpoints are `presets::Minimal` with `RelayMode::custom([local relay])` and no IP
//! transports. The ticket holds the relay URL and no direct addresses. The connection
//! must therefore go through the relay. Nothing is looked up through n0 DNS or pkarr.

use std::net::Ipv4Addr;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::presets::Minimal;
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl};
use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use iroh_tickets::endpoint::EndpointTicket;

use s1_transport::framed::{dial_once, serve_forever};

const ALPN: &[u8] = b"s1/relay-dial/0";
const ONLINE_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<()> {
    let mut config = ServerConfig::default();
    config.relay = Some(RelayConfig::new((Ipv4Addr::LOCALHOST, 0)));
    let relay = Server::spawn(config).await?;
    let http = relay.http_addr().context("relay has no HTTP address")?;
    let relay_url: RelayUrl = format!("http://{http}").parse()?;
    println!("local relay: {relay_url} (HTTP, no TLS, no QUIC address discovery)");

    let mode = RelayMode::custom([relay_url.clone()]);
    let server = Endpoint::builder(Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(mode.clone())
        .clear_ip_transports()
        .bind()
        .await?;
    let client = Endpoint::builder(Minimal)
        .relay_mode(mode)
        .clear_ip_transports()
        .bind()
        .await?;
    tokio::time::timeout(ONLINE_TIMEOUT, server.online())
        .await
        .context("server never reached the relay")?;
    tokio::time::timeout(ONLINE_TIMEOUT, client.online())
        .await
        .context("client never reached the relay")?;
    println!("both endpoints online through the local relay");

    // Ticket: relay URL only, no direct addresses.
    let ticket_addr = EndpointAddr::new(server.id()).with_relay_url(relay_url.clone());
    let ticket = EndpointTicket::new(ticket_addr).to_string();
    let parsed: EndpointTicket = ticket.parse()?;
    println!(
        "ticket: {} chars, {} direct address(es), relay = {}",
        ticket.len(),
        parsed.endpoint_addr().ip_addrs().count(),
        relay_url
    );

    let server_task = tokio::spawn(serve_forever(server.clone()));
    let (reply, elapsed, conn) = dial_once(&client, parsed.into(), ALPN).await?;
    println!(
        "reply = {:?} after {:?}",
        String::from_utf8_lossy(&reply),
        elapsed
    );
    for path in conn.paths().iter() {
        println!(
            "path: selected = {}, is_relay = {}, is_ip = {}, remote = {:?}",
            path.is_selected(),
            path.is_relay(),
            path.is_ip(),
            path.remote_addr()
        );
    }
    conn.close(0u32.into(), b"done");

    client.close().await;
    server.close().await;
    server_task.await??;
    relay.shutdown().await?;
    Ok(())
}
