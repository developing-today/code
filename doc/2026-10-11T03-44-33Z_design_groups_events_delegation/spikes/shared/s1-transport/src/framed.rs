//! Length-prefixed framing and one request/reply over a bidirectional stream, shared by the
//! `dial` and `relay` binaries.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};

pub async fn write_frame(send: &mut SendStream, payload: &[u8]) -> Result<()> {
    send.write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    send.write_all(payload).await?;
    Ok(())
}

pub async fn read_frame(recv: &mut RecvStream) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
    recv.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Answers one connection: reads one framed request and replies `echo: <request>`.
async fn handle(incoming: Incoming) -> Result<()> {
    let conn = incoming.await?;
    let (mut send, mut recv) = conn.accept_bi().await?;
    let request = read_frame(&mut recv).await?;
    let mut reply = b"echo: ".to_vec();
    reply.extend_from_slice(&request);
    write_frame(&mut send, &reply).await?;
    send.finish()?;
    conn.closed().await;
    Ok(())
}

/// Accepts connections until the endpoint closes. Each one is answered in its own task.
pub async fn serve_forever(endpoint: Endpoint) -> Result<()> {
    while let Some(incoming) = endpoint.accept().await {
        tokio::spawn(async move {
            if let Err(err) = handle(incoming).await {
                eprintln!("server: connection failed: {err:#}");
            }
        });
    }
    Ok(())
}

/// Dials `addr` on `alpn`, sends one framed request, and returns the reply, the time taken,
/// and the connection (still open, so the caller can inspect its paths).
pub async fn dial_once(
    client: &Endpoint,
    addr: EndpointAddr,
    alpn: &[u8],
) -> Result<(Vec<u8>, Duration, Connection)> {
    let started = Instant::now();
    let conn = client.connect(addr, alpn).await.context("connect")?;
    let (mut send, mut recv) = conn.open_bi().await?;
    write_frame(&mut send, b"hello from the dialer").await?;
    send.finish()?;
    let reply = read_frame(&mut recv).await?;
    Ok((reply, started.elapsed(), conn))
}
