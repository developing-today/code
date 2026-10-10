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
use rand::RngExt as _;
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
    world: Option<&str>,
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
    let client = WorldClient::connect(&endpoint, addr)
        .await?
        .with_world(world);
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
            world,
            addrs,
            no_relay,
            expires_in,
            uses,
        } => {
            crate::world_session::check_invite_bounds(expires_in, uses)
                .map_err(anyhow::Error::msg)?;
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client.invite(&admin_token, &name, expires_in, uses).await;
            client.close();
            endpoint.close().await;
            // Only the capability goes to stdout, so it can be captured.
            println!("{}", result?);
            Ok(())
        }
        WorldCommand::Install {
            node,
            module,
            admin_token,
            seed,
            world,
            addrs,
            no_relay,
        } => {
            let metadata = std::fs::metadata(&module)
                .with_context(|| format!("read module metadata {}", module.display()))?;
            anyhow::ensure!(
                metadata.len() <= crate::world_session::MAX_WORLD_MODULE_BYTES as u64,
                "module exceeds {} bytes",
                crate::world_session::MAX_WORLD_MODULE_BYTES
            );
            let wasm = std::fs::read(&module)
                .with_context(|| format!("read Wasm module {}", module.display()))?;
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client.install_wasm(&admin_token, &wasm, seed).await;
            client.close();
            endpoint.close().await;
            println!("{}", result?);
            Ok(())
        }
        WorldCommand::Records {
            node,
            capability,
            prefix,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client.records(&capability, prefix.as_deref()).await;
            client.close();
            endpoint.close().await;
            println!("{}", serde_json::to_string_pretty(&result?)?);
            Ok(())
        }
        WorldCommand::Mirror {
            node,
            capability,
            follow,
            dir,
            timeout_secs,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let ticket = client.records_ticket(&capability).await?;
            client.close();
            // Replication runs over iroh-docs, not the world session: the
            // replica keeps syncing with any host node for as long as it runs.
            let replica = Replica::start(dir.as_deref(), no_relay).await?;
            let timeout = std::time::Duration::from_secs(timeout_secs.max(1));
            let (doc, records) =
                crate::world_records::replicate(&replica.docs, &replica.blobs, &ticket, timeout)
                    .await?;
            println!("{}", serde_json::to_string_pretty(&records)?);
            if follow {
                use futures_lite::StreamExt as _;
                let mut events = doc.subscribe().await?;
                while let Some(event) = events.next().await {
                    if let iroh_docs::engine::LiveEvent::InsertRemote { .. } = event? {
                        // Coalesce bursts, then re-read.
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let records =
                            crate::world_records::read_records(&doc, &replica.blobs).await?;
                        println!("{}", serde_json::to_string_pretty(&records)?);
                    }
                }
            }
            endpoint.close().await;
            replica.shutdown().await;
            Ok(())
        }
        WorldCommand::Download {
            node,
            capability,
            output,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = async {
                let (_, active_module_hash, _) = client.info(&capability).await?;
                let module_hash =
                    active_module_hash.context("world has no installed module to download")?;
                let bytes = client.download_module(&capability, &module_hash).await?;
                tokio::fs::write(&output, &bytes)
                    .await
                    .with_context(|| format!("write module to {}", output.display()))?;
                println!("{module_hash}");
                Ok::<(), anyhow::Error>(())
            }
            .await;
            client.close();
            endpoint.close().await;
            result
        }
        WorldCommand::Compile {
            node,
            main_file,
            files,
            seed,
            admin_token,
            native,
            world,
            addrs,
            no_relay,
        } => {
            let mut paths = vec![("main.roc".to_owned(), main_file.clone())];
            for path in &files {
                let name = path
                    .file_name()
                    .context("a source file has no name")?
                    .to_string_lossy()
                    .into_owned();
                paths.push((name, path.clone()));
            }
            let mut sources = Vec::new();
            for (name, path) in paths {
                let content = tokio::fs::read_to_string(&path)
                    .await
                    .with_context(|| format!("read {}", path.display()))?;
                anyhow::ensure!(
                    content.len() <= crate::world_compile::MAX_COMPILE_FILE_BYTES,
                    "{} exceeds {} bytes",
                    path.display(),
                    crate::world_compile::MAX_COMPILE_FILE_BYTES
                );
                sources.push((name, content));
            }
            anyhow::ensure!(
                sources.iter().map(|(_, c)| c.len()).sum::<usize>()
                    <= crate::world_compile::MAX_COMPILE_TOTAL_BYTES,
                "the sources exceed {} bytes",
                crate::world_compile::MAX_COMPILE_TOTAL_BYTES
            );
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let seed = seed.unwrap_or_else(|| rand::rng().random::<u64>());
            let result = client.compile(&admin_token, sources, seed, native).await;
            client.close();
            endpoint.close().await;
            let (hash, _sequence, diagnostics) = result?;
            if !diagnostics.trim().is_empty() {
                eprintln!("{diagnostics}");
            }
            println!("{hash}");
            Ok(())
        }
        WorldCommand::Caps {
            node,
            capability,
            admin_token,
            grant,
            revoke,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = if grant.is_empty() && revoke.is_empty() {
                let capability = capability
                    .context("a guest capability (--capability) is needed to read the report")?;
                client.caps(&capability).await
            } else {
                let admin_token = admin_token
                    .context("the admin token is needed to change what the world may use")?;
                client.update_caps(&admin_token, &grant, &revoke).await
            };
            client.close();
            endpoint.close().await;
            println!("{}", serde_json::to_string_pretty(&result?)?);
            Ok(())
        }
        WorldCommand::Isolation {
            node,
            mode,
            admin_token,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client.set_isolation(&admin_token, &mode).await;
            client.close();
            endpoint.close().await;
            println!("{}", result?);
            Ok(())
        }
        WorldCommand::CrossWorldWrite {
            node,
            mode,
            admin_token,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client
                .set_cross_world_write(&admin_token, mode == "on")
                .await;
            client.close();
            endpoint.close().await;
            println!("{}", serde_json::to_string_pretty(&result?)?);
            Ok(())
        }
        WorldCommand::ForceCrossWorldWrite {
            node,
            admin_token,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client.force_cross_world_write(&admin_token).await;
            client.close();
            endpoint.close().await;
            println!("{}", serde_json::to_string_pretty(&result?)?);
            Ok(())
        }
        WorldCommand::FileScope {
            node,
            scope,
            write_override,
            admin_token,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client
                .set_file_scope(
                    &admin_token,
                    &scope,
                    write_override.as_deref().map(|mode| mode == "on"),
                )
                .await;
            client.close();
            endpoint.close().await;
            println!("{}", serde_json::to_string_pretty(&result?)?);
            Ok(())
        }
        WorldCommand::List {
            node,
            admin_token,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, None).await?;
            let result = client.list_worlds(&admin_token).await;
            client.close();
            endpoint.close().await;
            let (default, worlds) = result?;
            for (name, open) in worlds {
                let default_marker = if name == default { " (default)" } else { "" };
                println!(
                    "{name}{default_marker}\t{}",
                    if open { "open" } else { "closed" }
                );
            }
            Ok(())
        }
        WorldCommand::Create {
            node,
            name,
            admin_token,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, None).await?;
            let result = client.create_world(&admin_token, &name).await;
            client.close();
            endpoint.close().await;
            println!("{}", if result? { "created" } else { "exists" });
            Ok(())
        }
        WorldCommand::Join {
            node,
            capability,
            after,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
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

/// An in-memory iroh-docs replica: enough to sync and verify records
/// without writing anything to disk.
struct Replica {
    router: iroh::protocol::Router,
    endpoint: Endpoint,
    docs: iroh_docs::protocol::Docs,
    blobs: iroh_blobs::api::Store,
}

impl Replica {
    /// Start a replica. With `dir`, the node identity, blob store and docs
    /// database live under it, so the replica survives between runs and only
    /// fetches what changed; without it everything is in memory.
    async fn start(dir: Option<&std::path::Path>, no_relay: bool) -> Result<Self> {
        use iroh::endpoint::{RelayMode, presets};
        use iroh_blobs::store::mem::MemStore;
        use iroh_gossip::net::Gossip;

        let (key, blobs, docs) = match dir {
            Some(dir) => {
                tokio::fs::create_dir_all(dir.join("blobs")).await?;
                tokio::fs::create_dir_all(dir.join("docs")).await?;
                let key_path = dir.join("node.key");
                let key = load_or_create_keypair(&key_path.to_string_lossy()).await?;
                let store = iroh_blobs::store::fs::FsStore::load(dir.join("blobs")).await?;
                let blobs: iroh_blobs::api::Store = store.into();
                (
                    key,
                    blobs,
                    iroh_docs::protocol::Docs::persistent(dir.join("docs")),
                )
            }
            None => {
                let blobs: iroh_blobs::api::Store = MemStore::new().into();
                (
                    load_or_create_keypair(CLIENT_KEY_FILE).await?,
                    blobs,
                    iroh_docs::protocol::Docs::memory(),
                )
            }
        };
        let endpoint = if no_relay {
            Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .secret_key(key)
                .bind()
                .await?
        } else {
            Endpoint::builder(presets::N0)
                .secret_key(key)
                .bind()
                .await?
        };
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs = docs
            .spawn(endpoint.clone(), blobs.clone(), gossip.clone())
            .await?;
        let router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(iroh_docs::net::ALPN, docs.clone())
            .accept(
                iroh_blobs::ALPN,
                iroh_blobs::BlobsProtocol::new(&blobs, None),
            )
            .accept(iroh_gossip::net::GOSSIP_ALPN, gossip)
            .spawn();
        Ok(Self {
            router,
            endpoint,
            docs,
            blobs,
        })
    }

    async fn shutdown(self) {
        let _ = self.router.shutdown().await;
        self.endpoint.close().await;
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
