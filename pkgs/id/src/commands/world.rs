//! `id world`: invite to and join multiplayer worlds hosted over Iroh.
//!
//! Frames are the JSON protocol of [`crate::world_session`]. `join` prints
//! every server frame as one JSON line, so a terminal, script, or other
//! presentation can consume the same stream a browser would.

use std::net::SocketAddr;

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, TransportAddr,
    endpoint::{RelayMode, presets},
};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    CLIENT_KEY_FILE,
    cli::WorldCommand,
    is_node_id, load_or_create_keypair,
    world_net::{WorldClient, read_frame, write_frame},
};

/// Dial a world host and open one session.
async fn connect(
    node: &str,
    addrs: &[SocketAddr],
    no_relay: bool,
) -> Result<(Endpoint, WorldClient)> {
    if !is_node_id(node) {
        bail!("`{node}` is not a node ID (expected 64 hex characters)");
    }
    let id: EndpointId = node.parse().context("parse node ID")?;
    let key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    // Direct addresses need neither relay nor discovery, so keep the client
    // off the public network entirely in that case.
    let endpoint = if addrs.is_empty() {
        let mut builder = Endpoint::builder(presets::N0).secret_key(key);
        if no_relay {
            builder = builder.relay_mode(RelayMode::Disabled);
        }
        builder.bind().await?
    } else {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(key)
            .bind()
            .await?
    };
    let addr = EndpointAddr::from_parts(id, addrs.iter().copied().map(TransportAddr::Ip));
    let client = WorldClient::connect(&endpoint, addr).await?;
    Ok((endpoint, client))
}

/// Run an `id world` subcommand.
///
/// # Errors
///
/// Fails if the host is unreachable, refuses the invite, or the stream breaks.
pub async fn cmd_world(command: WorldCommand) -> Result<()> {
    match command {
        WorldCommand::Invite {
            node,
            admin_token,
            name,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay).await?;
            let result = client.invite(&admin_token, &name).await;
            client.close();
            endpoint.close().await;
            // Only the capability goes to stdout, so it can be captured.
            println!("{}", result?);
            Ok(())
        }
        WorldCommand::Join {
            node,
            capability,
            after,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay).await?;
            client
                .send_json(&serde_json::json!({
                    "type": "join",
                    "capability": capability,
                    "after": after,
                }))
                .await?;
            let result = pump(client).await;
            endpoint.close().await;
            result
        }
    }
}

/// Print server frames as JSON lines while sending stdin lines as chat.
async fn pump(client: WorldClient) -> Result<()> {
    let (conn, mut send, mut recv) = client.into_parts();
    let mut reader = tokio::spawn(async move {
        while let Some(body) = read_frame(&mut recv).await? {
            println!("{}", String::from_utf8_lossy(&body));
        }
        anyhow::Ok(())
    });
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = true;
    let result = loop {
        tokio::select! {
            line = lines.next_line(), if stdin_open => match line? {
                Some(line) => {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    if line == "/quit" { break Ok(()); }
                    let frame = match line.strip_prefix("/input ") {
                        Some(hex) => serde_json::json!({"type": "input", "data_hex": hex.trim()}),
                        None => serde_json::json!({"type": "chat", "text": line}),
                    };
                    if let Err(e) = write_frame(&mut send, frame.to_string().as_bytes()).await {
                        break Err(e);
                    }
                }
                None => stdin_open = false,
            },
            ended = &mut reader => {
                break ended.context("reader task")?;
            }
            _ = tokio::signal::ctrl_c() => break Ok(()),
        }
    };
    conn.close(0_u32.into(), b"done");
    reader.abort();
    result
}
