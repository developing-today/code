//! Spike S1 part (e): ticket-only dialing with `presets::Minimal` endpoints on localhost.
//!
//! `Minimal` sets only the crypto provider. It has no relay (RelayMode disabled) and no
//! address lookup, so a dialer's only addresses are the ones in the ticket. Each failure case
//! uses a fresh client endpoint. A client that has already dialed a node keeps that node's
//! addresses, which would hide the failure.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use anyhow::Result;
use iroh::endpoint::presets::Minimal;
use iroh::{Endpoint, EndpointAddr};
use iroh_tickets::endpoint::EndpointTicket;

use s1_transport::framed::{dial_once, serve_forever};

const ALPN: &[u8] = b"s1/ticket-dial/0";
const CASE_TIMEOUT: Duration = Duration::from_secs(60);

/// A localhost UDP port with nothing listening on it.
fn closed_port() -> Result<u16> {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(socket.local_addr()?.port())
}

async fn localhost_endpoint(alpns: Vec<Vec<u8>>) -> Result<Endpoint> {
    Ok(Endpoint::builder(Minimal)
        .alpns(alpns)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))?
        .bind()
        .await?)
}

#[tokio::main]
async fn main() -> Result<()> {
    let server = localhost_endpoint(vec![ALPN.to_vec()]).await?;
    let ticket = EndpointTicket::new(server.addr()).to_string();
    println!("server id: {}", server.id());
    println!(
        "ticket: {} chars, {} direct address(es)",
        ticket.len(),
        server.addr().ip_addrs().count()
    );
    let server_task = tokio::spawn(serve_forever(server.clone()));

    // 1. Ticket with a reachable direct address.
    let client = localhost_endpoint(vec![]).await?;
    let parsed: EndpointTicket = ticket.parse()?;
    match dial_once(&client, parsed.into(), ALPN).await {
        Ok((reply, elapsed, conn)) => {
            println!(
                "success: reply = {:?} after {:?}",
                String::from_utf8_lossy(&reply),
                elapsed
            );
            conn.close(0u32.into(), b"done");
        }
        Err(err) => println!("success case FAILED: {err:#}"),
    }
    client.close().await;

    // 2. Ticket with no addresses at all. Fresh client, so it has never seen the server.
    let client = localhost_endpoint(vec![]).await?;
    let no_addr: EndpointTicket = EndpointTicket::new(EndpointAddr::new(server.id()))
        .to_string()
        .parse()?;
    let started = Instant::now();
    match tokio::time::timeout(
        CASE_TIMEOUT,
        client.connect(EndpointAddr::from(no_addr), ALPN),
    )
    .await
    {
        Ok(Ok(_)) => println!("no-address case: unexpectedly connected"),
        Ok(Err(err)) => println!(
            "no-address case: failed after {:?}\n  display: {err}\n  debug: {err:?}",
            started.elapsed()
        ),
        Err(_) => println!("no-address case: no result after {CASE_TIMEOUT:?}"),
    }
    client.close().await;

    // 3. Ticket whose only address is a closed port. Fresh client.
    let client = localhost_endpoint(vec![]).await?;
    let dead = EndpointAddr::new(server.id())
        .with_ip_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, closed_port()?)));
    let started = Instant::now();
    match tokio::time::timeout(CASE_TIMEOUT, client.connect(dead, ALPN)).await {
        Ok(Ok(_)) => println!("dead-address case: unexpectedly connected"),
        Ok(Err(err)) => println!(
            "dead-address case: failed after {:?}\n  display: {err}",
            started.elapsed()
        ),
        Err(_) => println!("dead-address case: no result after {CASE_TIMEOUT:?}"),
    }
    client.close().await;

    server.close().await;
    server_task.await??;
    Ok(())
}
