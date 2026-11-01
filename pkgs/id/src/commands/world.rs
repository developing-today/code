//! `id world`: invite to and join multiplayer worlds hosted over Iroh.
//!
//! Frames are the JSON protocol of [`crate::world_session`]. `join` prints
//! every server frame as one JSON line, so a terminal, script, or other
//! presentation can consume the same stream a browser would.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, TransportAddr,
    endpoint::{RelayMode, presets},
};
use rand::RngExt as _;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{
    CLIENT_KEY_FILE,
    cli::{WorldCommand, WorldVia},
    frames::{read_frame_from, write_frame_to},
    is_node_id, load_or_create_keypair,
    world_net::WorldClient,
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
    let endpoint = client_endpoint(addrs, no_relay).await?;
    let addr = EndpointAddr::from_parts(id, addrs.iter().copied().map(TransportAddr::Ip));
    let client = WorldClient::connect(&endpoint, addr)
        .await?
        .with_world(world);
    Ok((endpoint, client))
}

/// A client endpoint for reaching a host: direct addresses need neither
/// relay nor discovery, so keep it off the public network in that case.
async fn client_endpoint(addrs: &[SocketAddr], no_relay: bool) -> Result<Endpoint> {
    let key = load_or_create_keypair(CLIENT_KEY_FILE).await?;
    let endpoint = if addrs.is_empty() {
        crate::connectivity::public_builder(crate::connectivity::RelayPolicy::from_no_relay(
            no_relay,
        ))
        .secret_key(key)
        .bind()
        .await?
    } else {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .secret_key(key)
            .bind()
            .await?
    };
    Ok(endpoint)
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
            role,
            world,
            addrs,
            no_relay,
            expires_in,
            uses,
        } => {
            crate::world_session::check_invite_bounds(expires_in, uses)
                .map_err(anyhow::Error::msg)?;
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = client
                .invite_with_role(&admin_token, &name, expires_in, uses, role.as_deref())
                .await;
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
            ceiling,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = if let Some(ceiling) = ceiling {
                let admin_token = admin_token
                    .context("the admin token is needed to change the capability ceiling")?;
                let ceiling_res = client.set_caps_ceiling(&admin_token, &ceiling).await?;
                if !grant.is_empty() || !revoke.is_empty() {
                    let updated = client.update_caps(&admin_token, &grant, &revoke).await?;
                    serde_json::to_value(updated)?
                } else {
                    ceiling_res
                }
            } else if !grant.is_empty() || !revoke.is_empty() {
                let admin_token = admin_token
                    .context("the admin token is needed to change what the world may use")?;
                let updated = client.update_caps(&admin_token, &grant, &revoke).await?;
                serde_json::to_value(updated)?
            } else {
                let capability = capability
                    .context("a guest capability (--capability) is needed to read the report")?;
                let report = client.caps(&capability).await?;
                serde_json::to_value(report)?
            };
            client.close();
            endpoint.close().await;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        WorldCommand::Profile {
            node,
            admin_token,
            capability,
            wasm_fuel,
            wasm_memory_bytes,
            wasm_message_bytes,
            native_memory_bytes,
            native_cpu_seconds,
            native_call_deadline_ms,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let has_updates = wasm_fuel.is_some()
                || wasm_memory_bytes.is_some()
                || wasm_message_bytes.is_some()
                || native_memory_bytes.is_some()
                || native_cpu_seconds.is_some()
                || native_call_deadline_ms.is_some();
            let result = if has_updates {
                let admin_token = admin_token.context(
                    "the admin token is needed to configure the world's execution profile",
                )?;
                let profile = crate::world_limits::WorldExecutionProfile {
                    wasm_fuel,
                    wasm_memory_bytes,
                    wasm_message_bytes,
                    native_memory_bytes,
                    native_cpu_seconds,
                    native_call_deadline_ms,
                };
                client.set_execution_profile(&admin_token, profile).await
            } else {
                client
                    .execution_profile(admin_token.as_deref(), capability.as_deref())
                    .await
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
        WorldCommand::New { dir, name } => scaffold_world(&dir, &name),
        WorldCommand::Doctor {
            roc_bin,
            platform,
            journal,
            ceiling,
            profile,
            json,
        } => {
            let platform_dir = platform
                .or_else(|| std::env::var("ID_ROC_PLATFORM").ok().map(PathBuf::from))
                .or_else(|| {
                    [
                        PathBuf::from("examples/roc-world"),
                        PathBuf::from("../examples/roc-world"),
                    ]
                    .into_iter()
                    .find(|dir| dir.is_dir())
                });
            // The ceiling/profile checks take their values from the
            // --ceiling / --profile flags; without them the checks report
            // that none is configured (they are not silently passed off as
            // a live world's values).
            let profile = match profile {
                Some(path) => {
                    let text = std::fs::read_to_string(&path)
                        .with_context(|| format!("read execution profile {}", path.display()))?;
                    Some(
                        serde_json::from_str::<crate::world_limits::WorldExecutionProfile>(
                            &text,
                        )
                        .with_context(|| {
                            format!("parse execution profile {}", path.display())
                        })?,
                    )
                }
                None => None,
            };
            let inputs = crate::world_doctor::DoctorInputs {
                roc_bin,
                roc_resolved: None,
                platform_dir,
                caps_ceiling: ceiling,
                profile,
                journal,
                now_ms: Some(crate::time::unix_ms()),
            };
            let report = crate::world_doctor::run(&inputs);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render_text());
            }
            // Warn findings print but do not fail the command; only Fail
            // findings exit nonzero, counted by failure_count().
            report.exit_result()
        }
        WorldCommand::Diff {
            node,
            file,
            capability,
            world,
            addrs,
            no_relay,
        } => {
            let manifest = parse_manifest_file(&file)?;
            let (endpoint, client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let state = current_world_state(&client, &capability, None).await?;
            client.close();
            endpoint.close().await;
            print!("{}", crate::world_manifest::diff(&manifest, &state));
            Ok(())
        }
        WorldCommand::Apply {
            node,
            file,
            admin_token,
            capability,
            dry_run,
            world,
            addrs,
            no_relay,
        } => {
            let manifest = parse_manifest_file(&file)?;
            let (endpoint, client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let state = current_world_state(&client, &capability, Some(&admin_token)).await?;
            let steps = crate::world_manifest::plan(&manifest, &state);
            if steps.is_empty() {
                println!("no drift");
            } else {
                println!(
                    "{}",
                    steps
                        .iter()
                        .map(crate::world_manifest::ApplyStep::kind)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                if !dry_run {
                    let applied = apply_steps(&client, &admin_token, &capability, &steps).await?;
                    if applied == steps.len() {
                        println!("applied {} step(s)", applied);
                    } else {
                        println!(
                            "applied {} of {} step(s); the rest were reported but not applied",
                            applied,
                            steps.len()
                        );
                    }
                } else {
                    println!("dry run: nothing changed");
                }
            }
            client.close();
            endpoint.close().await;
            Ok(())
        }
        WorldCommand::Contract { check: None } => {
            print!("{}", crate::shell_contract::SHELL_CONTRACT_JSON);
            Ok(())
        }
        WorldCommand::Contract { check: Some(path) } => {
            let text = if path.as_os_str() == "-" {
                std::io::read_to_string(std::io::stdin()).context("read transcript")?
            } else {
                std::fs::read_to_string(&path)
                    .with_context(|| format!("read transcript {}", path.display()))?
            };
            let checked = check_contract(&text)?;
            println!("{checked}");
            Ok(())
        }
        #[cfg(feature = "sandbox")]
        WorldCommand::Measure {
            module,
            seed,
            inputs,
            width,
            height,
        } => {
            let wasm = std::fs::read(&module)
                .with_context(|| format!("read Wasm module {}", module.display()))?;
            let report = measure_module(&wasm, seed, &inputs, width.zip(height))?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        WorldCommand::Logs {
            node,
            capability,
            seconds,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            client
                .send_json(&serde_json::json!({
                    "type": "join",
                    "capability": capability,
                    "after": 0,
                }))
                .await?;
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_secs(seconds.max(1));
            let result = loop {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break Ok(());
                }
                match tokio::time::timeout(remaining, client.recv_json()).await {
                    Err(_) => break Ok(()),
                    Ok(Err(error)) => break Err(error),
                    Ok(Ok(None)) => break Ok(()),
                    Ok(Ok(Some(frame))) => {
                        if frame["type"] == "event" {
                            println!("{}", serde_json::to_string(&frame)?);
                        } else if frame["type"] == "error" {
                            break Err(anyhow::anyhow!(
                                "{}",
                                frame["message"]
                                    .as_str()
                                    .unwrap_or("world refused the event tail")
                            ));
                        }
                    }
                }
            };
            client.close();
            endpoint.close().await;
            result
        }
        WorldCommand::Stats {
            node,
            capability,
            world,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, world.as_deref()).await?;
            let result = async {
                let info = client.info_details(&capability).await?;
                let caps = client.caps(&capability).await?;
                let profile = client.execution_profile(None, Some(&capability)).await?;
                // Host metrics (F11.7) ride along when the host knows the
                // metrics frame; an older host simply omits the object.
                let metrics = client.metrics(&capability).await.ok();
                Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
                    "world_id": info.world_id,
                    "active_module_hash": info.active_module_hash,
                    "active_module_provenance": info.active_module_provenance,
                    "current_sequence": info.current_sequence,
                    "caps": caps,
                    "profile": profile,
                    "metrics": metrics,
                }))
            }
            .await;
            client.close();
            endpoint.close().await;
            println!("{}", serde_json::to_string_pretty(&result?)?);
            Ok(())
        }
        WorldCommand::Record {
            node,
            capability,
            out,
            world,
            addrs,
            no_relay,
        } => {
            // Boxed so that `cmd_world`, already a large state machine, does not
            // grow by the recording's futures.
            Box::pin(record_command(
                &node,
                &capability,
                out.as_deref(),
                world.as_deref(),
                &addrs,
                no_relay,
            ))
            .await
        }
        #[cfg(feature = "sandbox")]
        WorldCommand::Replay {
            module,
            fixture,
            seed,
            allow_partial,
        } => replay_fixture(&module, &fixture, seed, allow_partial),
        #[cfg(feature = "sandbox")]
        WorldCommand::Verify {
            module,
            fixture,
            seed,
            allow_partial,
        } => verify_fixture(&module, &fixture, seed, allow_partial),
        WorldCommand::List {
            node,
            admin_token,
            json,
            addrs,
            no_relay,
        } => {
            let (endpoint, mut client) = connect(&node, &addrs, no_relay, None).await?;
            let result = client.list_worlds(&admin_token).await;
            client.close();
            endpoint.close().await;
            let (default, worlds) = result?;
            if json {
                let worlds: Vec<crate::world_hub::WorldInfo> = worlds
                    .into_iter()
                    .map(|(name, open)| crate::world_hub::WorldInfo { name, open })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&crate::world_hub::directory_json(
                        &worlds, &default
                    ))?
                );
            } else {
                for (name, open) in worlds {
                    let default_marker = if name == default { " (default)" } else { "" };
                    println!(
                        "{name}{default_marker}\t{}",
                        if open { "open" } else { "closed" }
                    );
                }
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
            command_mode,
            world,
            addrs,
            no_relay,
            via,
            ssh_host_key,
        } => {
            let join = serde_json::json!({
                "type": "join",
                "capability": capability,
                "after": after,
            });
            // The same routed bytes on every transport.
            let first = crate::world_net::route_frame(&join, world.as_deref());
            match via {
                WorldVia::Iroh => {
                    if ssh_host_key.is_some() {
                        bail!("--ssh-host-key needs --via ssh");
                    }
                    let (endpoint, client) =
                        connect(&node, &addrs, no_relay, world.as_deref()).await?;
                    let (conn, send, recv) = client.into_parts();
                    let result = pump(recv, send, &first, command_mode, &capability).await;
                    conn.close(0_u32.into(), b"done");
                    endpoint.close().await;
                    result
                }
                WorldVia::Ssh => {
                    if !addrs.is_empty() || no_relay {
                        bail!("--addr and --no-relay apply to --via iroh only");
                    }
                    join_over_ssh(&node, ssh_host_key, &first, command_mode, &capability).await
                }
            }
        }
        WorldCommand::SshTunnel {
            node,
            listen,
            stdio,
            addrs,
            no_relay,
        } => ssh_tunnel(&node, listen.as_deref(), stdio, &addrs, no_relay).await,
        WorldCommand::Fork {
            source,
            dir,
            name,
            template,
            json,
        } => fork_world_on_disk(&source, &dir, name.as_deref(), template, json),
        WorldCommand::Provenance { dir, json } => print_world_provenance(&dir, json),
    }
}

/// Read a world directory's journal entries for inspection, without
/// opening the journal for append. A torn final fragment (a crash
/// mid-append, never acknowledged) is skipped, mirroring
/// `FileJournal::open`; any other unparseable line is corruption.
fn read_world_journal(dir: &std::path::Path) -> Result<Vec<crate::world::JournalEntry>> {
    let path = dir.join("journal.jsonl");
    let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let mut entries = Vec::new();
    let lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<crate::world::JournalEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(error) => {
                let is_torn_tail = index == lines.len() - 1 && bytes.last() != Some(&b'\n');
                if is_torn_tail {
                    break;
                }
                bail!("{} is corrupt: {error}", path.display());
            }
        }
    }
    Ok(entries)
}

/// A world directory's provenance, read from its journal: the `Created`
/// header's id plus the `Forked` entry a fork appends, if any.
struct WorldProvenance {
    world_id: String,
    forked: Option<(String, String, bool, u64)>,
}

fn world_provenance(dir: &std::path::Path) -> Result<WorldProvenance> {
    let entries = read_world_journal(dir)?;
    let Some(crate::world::JournalEntry::Created { world_id, .. }) = entries.first() else {
        bail!(
            "{} has no Created header",
            dir.join("journal.jsonl").display()
        );
    };
    let world_id = world_id.clone();
    let forked = entries.iter().find_map(|entry| match entry {
        crate::world::JournalEntry::Forked {
            from_world_id,
            from_module_hash,
            template,
            at_sequence,
        } => Some((
            from_world_id.clone(),
            from_module_hash.clone(),
            *template,
            *at_sequence,
        )),
        _ => None,
    });
    Ok(WorldProvenance { world_id, forked })
}

/// `id world provenance`: print a world directory's provenance, so a
/// template fork is identifiable without opening the world.
fn print_world_provenance(dir: &std::path::Path, json: bool) -> Result<()> {
    let provenance = world_provenance(dir)?;
    if json {
        let (from_world_id, from_module_hash, template, at_sequence) = match &provenance.forked {
            Some((from, hash, template, at)) => {
                (Some(from.clone()), Some(hash.clone()), *template, Some(*at))
            }
            None => (None, None, false, None),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "world_id": provenance.world_id,
                "dir": dir.display().to_string(),
                "forked_from": from_world_id,
                "from_module_hash": from_module_hash,
                "template": template,
                "at_sequence": at_sequence,
            }))?
        );
    } else {
        println!("world: {}", provenance.world_id);
        match &provenance.forked {
            Some((from, hash, template, at)) => {
                println!("forked_from: {from}");
                println!("from_module_hash: {hash}");
                println!("template: {template}");
                println!("at_sequence: {at}");
            }
            None => println!("forked_from: none"),
        }
    }
    Ok(())
}

/// `id world fork`: wire the store-level `fork_world` (F12.2) into the
/// CLI. The fork's provenance is read back from the new journal and
/// printed, so the caller sees exactly what was recorded.
fn fork_world_on_disk(
    source: &std::path::Path,
    dir: &std::path::Path,
    name: Option<&str>,
    template: bool,
    json: bool,
) -> Result<()> {
    let to_world_id = match name {
        Some(name) => name.to_owned(),
        None => dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .context("the destination directory has no name; pass --name")?,
    };
    crate::world_hub::validate_world_name(&to_world_id)?;
    if dir.exists() {
        anyhow::ensure!(
            std::fs::read_dir(dir)?.next().is_none(),
            "{} is not empty",
            dir.display()
        );
    }
    let from_world_id = world_provenance(source)?.world_id;
    crate::world_store::fork_world(
        source,
        dir,
        &crate::world_store::ForkSpec {
            from_world_id,
            to_world_id,
            template,
        },
    )?;
    if json {
        print_world_provenance(dir, true)
    } else {
        println!("forked {}", dir.display());
        print_world_provenance(dir, false)
    }
}

/// `id world join --via ssh`: the frame session over an SSH exec channel.
#[cfg(feature = "ssh")]
async fn join_over_ssh(
    target: &str,
    ssh_host_key: Option<String>,
    first: &str,
    command_mode: bool,
    capability: &str,
) -> Result<()> {
    use crate::world_ssh::{HostKeyCheck, SshTarget, connect_frames};
    let target: SshTarget = target.parse()?;
    let check = match ssh_host_key {
        Some(fingerprint) => HostKeyCheck::Fingerprint(fingerprint),
        None => {
            let home = std::env::var_os("HOME")
                .context("no HOME for ~/.ssh/known_hosts; pass --ssh-host-key")?;
            HostKeyCheck::KnownHosts(PathBuf::from(home).join(".ssh").join("known_hosts"))
        }
    };
    let (_connection, stream) = connect_frames(&target, check).await?;
    let (recv, send) = tokio::io::split(stream);
    pump(recv, send, first, command_mode, capability).await
}

/// `id world ssh-tunnel`: forward a local TCP listener (or stdio, as an SSH
/// ProxyCommand) over iroh to the host's world SSH server (F13.3). The
/// tunnel carries SSH bytes only; the `ssh` client still authenticates to
/// the host inside it.
#[cfg(feature = "ssh")]
async fn ssh_tunnel(
    node: &str,
    listen: Option<&str>,
    stdio: bool,
    addrs: &[SocketAddr],
    no_relay: bool,
) -> Result<()> {
    if !is_node_id(node) {
        bail!("`{node}` is not a node ID (expected 64 hex characters)");
    }
    let id: EndpointId = node.parse().context("parse node ID")?;
    let endpoint = client_endpoint(addrs, no_relay).await?;
    let addr = EndpointAddr::from_parts(id, addrs.iter().copied().map(TransportAddr::Ip));
    let conn = crate::connectivity::connect(&endpoint, addr, crate::world_ssh::SSH_TUNNEL_ALPN)
        .await
        .context("connect ssh tunnel")?;
    if stdio {
        let (send, recv) = conn.open_bi().await.context("open tunnel stream")?;
        let mut tunnel = tokio::io::join(recv, send);
        let mut local = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
        tokio::io::copy_bidirectional(&mut local, &mut tunnel)
            .await
            .context("ssh tunnel stdio")?;
        conn.close(0_u32.into(), b"done");
        endpoint.close().await;
        return Ok(());
    }
    let listen = listen.unwrap_or("127.0.0.1:2222");
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("listen on {listen}"))?;
    eprintln!(
        "ssh tunnel: {} -> world host; connect with `ssh -p {} <world>@{}` (password: a guest capability)",
        listener.local_addr()?,
        listener.local_addr()?.port(),
        listener.local_addr()?.ip()
    );
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (mut tcp, _) = accepted?;
                let conn = conn.clone();
                tokio::spawn(async move {
                    let Ok((send, recv)) = conn.open_bi().await else { return; };
                    let mut tunnel = tokio::io::join(recv, send);
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut tunnel).await;
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    conn.close(0_u32.into(), b"done");
    endpoint.close().await;
    Ok(())
}

#[cfg(not(feature = "ssh"))]
async fn ssh_tunnel(
    _node: &str,
    _listen: Option<&str>,
    _stdio: bool,
    _addrs: &[SocketAddr],
    _no_relay: bool,
) -> Result<()> {
    bail!("this build of id has no SSH support (cargo feature `ssh`)")
}

#[cfg(not(feature = "ssh"))]
async fn join_over_ssh(
    _target: &str,
    _ssh_host_key: Option<String>,
    _first: &str,
    _command_mode: bool,
    _capability: &str,
) -> Result<()> {
    bail!("this build of id has no SSH support (cargo feature `ssh`)")
}

/// Parse a `world.toml` file into a validated manifest.
fn parse_manifest_file(file: &std::path::Path) -> Result<crate::world_manifest::WorldManifest> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("read world manifest {}", file.display()))?;
    crate::world_manifest::parse(&text)
}

/// Fetch the current world state the manifest is compared against: the active
/// module hash, the grants, the ceiling, the execution profile, the pending
/// schedules and the install seed. The host returns the seed only to a caller
/// presenting the admin token (#240), so a capability-only read (diff) plans
/// with seed 0 — a seed alone never plans a step, and apply always reads with
/// the admin token.
async fn current_world_state(
    client: &WorldClient,
    capability: &str,
    admin_token: Option<&str>,
) -> Result<crate::world_manifest::WorldState> {
    // One fresh session per read: admin and query frames are served one
    // operation per stream, so sequencing these reads on one stream would
    // talk to a session the host already finished.
    let info = client
        .fresh_session()
        .await?
        .info_details(capability)
        .await?;
    let caps: crate::world_caps::CapsReport =
        serde_json::from_value(client.fresh_session().await?.caps(capability).await?)
            .context("host returned a malformed capability report")?;
    let profile: crate::world_limits::WorldExecutionProfile = serde_json::from_value(
        client
            .fresh_session()
            .await?
            .execution_profile(None, Some(capability))
            .await?,
    )
    .context("host returned a malformed execution profile")?;
    // The report carries the world's own ceiling as a list; an unrestricted
    // ceiling is the whole catalog, which we fold back to "no ceiling".
    let ceiling = if caps.ceiling
        == crate::world_caps::catalog_names()
            .into_iter()
            .collect::<Vec<_>>()
    {
        None
    } else {
        Some(caps.ceiling.clone())
    };
    let (seed, schedules) = client
        .fresh_session()
        .await?
        .manifest_state(admin_token, Some(capability))
        .await?;
    Ok(crate::world_manifest::WorldState {
        module_hash: info.active_module_hash,
        seed: seed.unwrap_or(0),
        grants: caps.granted,
        ceiling,
        profile,
        schedules,
    })
}

/// Whether [`apply_steps`] actually applies this step over the wire. Every
/// step kind is wired since #807 (module install and schedules included),
/// so every step counts toward the "applied N step(s)" summary (#818). The
/// predicate stays explicit per kind so a future reported-only step cannot
/// silently claim credit: list it here only when its apply path lands.
fn step_is_applied(step: &crate::world_manifest::ApplyStep) -> bool {
    use crate::world_manifest::ApplyStep;
    matches!(
        step,
        ApplyStep::InstallModule { .. }
            | ApplyStep::SetGrants { .. }
            | ApplyStep::SetCeiling { .. }
            | ApplyStep::SetProfile { .. }
            | ApplyStep::ScheduleAt { .. }
            | ApplyStep::CancelSchedule { .. }
    )
}

/// Apply every reconciliation step over the wire through the admin
/// operations: module install (compile a source, or download a known hash
/// and re-install its bytes), grants, ceiling, profile, and schedule
/// set/cancel — each journaled by the host. Returns how many of the steps
/// were actually applied, so the caller's summary cannot claim credit for
/// any step whose apply path is missing (#818).
///
/// The steps run in an order that makes the plan's result reachable, not
/// strictly in plan order (#807): schedule cancels first, so a manifest
/// that replaces ids at the pending-schedule capacity frees the slots its
/// additions need; the execution profile next, so a module installed in
/// the same apply is instantiated under the manifest's profile, not the
/// previous one (a profile applies on the next install); grants and
/// ceiling next, so a module compiled in the same apply may import the
/// capabilities the manifest grants; then the module install; schedule
/// additions last.
///
/// Each operation runs on a fresh session (see
/// [`WorldClient::fresh_session`]): the host serves admin frames one
/// operation per stream, so a second operation on the same stream would
/// write to a session that no longer has a receiver.
/// Order steps for reachability (see [`apply_steps`]): cancellations
/// first, then profile, grants and ceiling, then the module install,
/// and schedule additions last. The sort is stable, so steps of the
/// same kind keep their plan order.
fn ordered_steps<'a>(
    steps: &'a [crate::world_manifest::ApplyStep],
) -> Vec<&'a crate::world_manifest::ApplyStep> {
    use crate::world_manifest::ApplyStep;
    fn rank(step: &ApplyStep) -> u8 {
        match step {
            ApplyStep::CancelSchedule { .. } => 0,
            ApplyStep::SetProfile { .. } => 1,
            ApplyStep::SetGrants { .. } => 2,
            ApplyStep::SetCeiling { .. } => 3,
            ApplyStep::InstallModule { .. } => 4,
            ApplyStep::ScheduleAt { .. } => 5,
        }
    }
    let mut ordered: Vec<&ApplyStep> = steps.iter().collect();
    ordered.sort_by_key(|step| rank(step));
    ordered
}

async fn apply_steps(
    client: &WorldClient,
    admin_token: &str,
    capability: &str,
    steps: &[crate::world_manifest::ApplyStep],
) -> Result<usize> {
    use crate::world_manifest::ApplyStep;
    let applied = steps.iter().filter(|step| step_is_applied(step)).count();
    let ordered = ordered_steps(steps);
    for step in ordered {
        match step {
            ApplyStep::SetGrants { grants } => {
                let current: crate::world_caps::CapsReport =
                    serde_json::from_value(client.fresh_session().await?.caps(capability).await?)
                        .context("host returned a malformed capability report")?;
                let to_revoke: Vec<String> = current
                    .granted
                    .iter()
                    .filter(|name| !grants.contains(name))
                    .cloned()
                    .collect();
                let to_grant: Vec<String> = grants
                    .iter()
                    .filter(|name| !current.granted.contains(name))
                    .cloned()
                    .collect();
                if !to_grant.is_empty() || !to_revoke.is_empty() {
                    client
                        .fresh_session()
                        .await?
                        .update_caps(admin_token, &to_grant, &to_revoke)
                        .await?;
                }
            }
            ApplyStep::SetCeiling { ceiling } => {
                // `None` means "no ceiling": restore the whole catalog.
                let names = ceiling.clone().unwrap_or_else(|| {
                    crate::world_caps::catalog_names()
                        .into_iter()
                        .collect::<Vec<_>>()
                });
                client
                    .fresh_session()
                    .await?
                    .set_caps_ceiling(admin_token, &names)
                    .await?;
            }
            ApplyStep::SetProfile { profile } => {
                client
                    .fresh_session()
                    .await?
                    .set_execution_profile(admin_token, *profile)
                    .await?;
            }
            ApplyStep::InstallModule { hash, source, seed } => {
                if let Some(source) = source {
                    let sources = read_compile_sources(source)?;
                    client
                        .fresh_session()
                        .await?
                        .compile(admin_token, sources, *seed, false)
                        .await?;
                } else if let Some(hash) = hash {
                    // The module is already on the host: fetch its bytes by
                    // hash and install them under the manifest's seed.
                    let wasm = client
                        .fresh_session()
                        .await?
                        .download_module_admin(admin_token, hash)
                        .await?;
                    client
                        .fresh_session()
                        .await?
                        .install_wasm(admin_token, &wasm, *seed)
                        .await?;
                } else {
                    anyhow::bail!("install_module step names neither a hash nor a source");
                }
            }
            ApplyStep::ScheduleAt { id, at_ms } => {
                client
                    .fresh_session()
                    .await?
                    .set_schedule(admin_token, id, *at_ms)
                    .await?;
            }
            ApplyStep::CancelSchedule { id } => {
                client
                    .fresh_session()
                    .await?
                    .cancel_schedule(admin_token, id)
                    .await?;
            }
        }
    }
    Ok(applied)
}

/// Read the Roc sources a manifest `module_source` names, in the shape the
/// host compile operation expects: the main file first (always sent as
/// `main.roc`), then any sibling `.roc` files when the source is a
/// directory. Size bounds mirror `id world compile`.
fn read_compile_sources(source: &std::path::Path) -> Result<Vec<(String, String)>> {
    let mut paths: Vec<(String, std::path::PathBuf)> = Vec::new();
    if source.is_dir() {
        paths.push(("main.roc".to_owned(), source.join("main.roc")));
        let mut siblings: Vec<std::path::PathBuf> = std::fs::read_dir(source)
            .with_context(|| format!("read source directory {}", source.display()))?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "roc")
                    && path.file_name().is_some_and(|name| name != "main.roc")
            })
            .collect();
        siblings.sort();
        for path in siblings {
            let name = path
                .file_name()
                .context("a source file has no name")?
                .to_string_lossy()
                .into_owned();
            paths.push((name, path));
        }
    } else {
        paths.push(("main.roc".to_owned(), source.to_path_buf()));
    }
    let mut sources = Vec::new();
    let mut total = 0usize;
    for (name, path) in paths {
        let content =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        anyhow::ensure!(
            content.len() <= crate::world_compile::MAX_COMPILE_FILE_BYTES,
            "{} exceeds {} bytes",
            path.display(),
            crate::world_compile::MAX_COMPILE_FILE_BYTES
        );
        total += content.len();
        sources.push((name, content));
    }
    anyhow::ensure!(
        total <= crate::world_compile::MAX_COMPILE_TOTAL_BYTES,
        "the sources exceed {} bytes",
        crate::world_compile::MAX_COMPILE_TOTAL_BYTES
    );
    Ok(sources)
}

/// Record a world to a fixture: the file at `out`, or standard output.
///
/// # Errors
///
/// A host that cannot be reached, a refused capability, or a file that cannot
/// be written.
async fn record_command(
    node: &str,
    capability: &str,
    out: Option<&std::path::Path>,
    world: Option<&str>,
    addrs: &[SocketAddr],
    no_relay: bool,
) -> Result<()> {
    let (endpoint, mut client) = connect(node, addrs, no_relay, world).await?;
    let result = crate::world_fixture::record(&mut client, capability).await;
    client.close();
    endpoint.close().await;
    let fixture = result?;
    let text = crate::world_fixture::encode(&fixture.header, &fixture.events);
    match out {
        Some(path) => {
            tokio::fs::write(path, &text)
                .await
                .with_context(|| format!("write fixture to {}", path.display()))?;
            eprintln!(
                "recorded {} events through sequence {} ({}) to {}",
                fixture.events.len(),
                fixture.header.current_sequence,
                if fixture.header.complete {
                    "complete"
                } else {
                    "partial"
                },
                path.display()
            );
        }
        None => print!("{text}"),
    }
    Ok(())
}

/// Replay a fixture offline and print the verdict, the records, and the view.
///
/// The verdict line comes first. Records that differ from the fixture's make
/// the command fail, after the expected and replayed records are printed.
///
/// # Errors
///
/// An unreadable or oversized file, a module the fixture was not recorded
/// under, a refused replay, or records that differ from the fixture's.
#[cfg(feature = "sandbox")]
fn replay_fixture(
    module: &std::path::Path,
    fixture_path: &std::path::Path,
    seed: u64,
    allow_partial: bool,
) -> Result<()> {
    use crate::world_fixture::Verdict;

    let wasm = read_bounded(
        module,
        crate::world_session::MAX_WORLD_MODULE_BYTES as u64,
        "module",
    )?;
    let text = String::from_utf8(read_bounded(
        fixture_path,
        crate::world_fixture::MAX_FIXTURE_BYTES,
        "fixture",
    )?)
    .context("the fixture is not UTF-8")?;
    let fixture = crate::world_fixture::decode(&text)?;
    crate::world_fixture::check_module(&fixture.header, &wasm)?;
    let runner = crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())?;
    let replayed = crate::world_fixture::replay(&runner, seed, &fixture, allow_partial)?;
    match &replayed.verdict {
        Verdict::Match => println!("records match"),
        Verdict::Mismatch => println!("records differ"),
        Verdict::NotCompared(reason) => println!("records not compared ({reason})"),
    }
    if let Some(records) = &replayed.records {
        let value: serde_json::Value =
            serde_json::from_str(records).context("the replayed records")?;
        println!("records: {}", serde_json::to_string_pretty(&value)?);
    }
    if let Some(view) = &replayed.view {
        println!("view:\n{view}");
    }
    if replayed.verdict == Verdict::Mismatch {
        if let Some(expected) = &fixture.header.records {
            println!(
                "expected records: {}",
                serde_json::to_string_pretty(expected)?
            );
        }
        bail!("the replayed records differ from the fixture's");
    }
    Ok(())
}

/// Verify a fixture's determinism offline and print the verdict.
///
/// The verdict line comes first. When the two replays diverge, the first
/// diverging record is printed and the command fails, after the first
/// replay's records and view are printed.
///
/// # Errors
///
/// An unreadable or oversized file, a module the fixture was not recorded
/// under, a refused replay, or two replays that diverge.
#[cfg(feature = "sandbox")]
fn verify_fixture(
    module: &std::path::Path,
    fixture_path: &std::path::Path,
    seed: u64,
    allow_partial: bool,
) -> Result<()> {
    let wasm = read_bounded(
        module,
        crate::world_session::MAX_WORLD_MODULE_BYTES as u64,
        "module",
    )?;
    let text = String::from_utf8(read_bounded(
        fixture_path,
        crate::world_fixture::MAX_FIXTURE_BYTES,
        "fixture",
    )?)
    .context("the fixture is not UTF-8")?;
    let fixture = crate::world_fixture::decode(&text)?;
    crate::world_fixture::check_module(&fixture.header, &wasm)?;
    let runner = crate::sandbox::Sandbox::compile(&wasm, crate::sandbox::SandboxLimits::default())?;
    let verification = crate::world_fixture::verify(&runner, seed, &fixture, allow_partial)?;
    match &verification.divergence {
        None => println!(
            "records verify (two replays agree at all {} steps)",
            verification.steps
        ),
        Some(divergence) => println!("records diverge ({divergence})"),
    }
    if let Some(records) = &verification.first.records {
        let value: serde_json::Value =
            serde_json::from_str(records).context("the replayed records")?;
        println!("records: {}", serde_json::to_string_pretty(&value)?);
    }
    if let Some(view) = &verification.first.view {
        println!("view:\n{view}");
    }
    if let Some(divergence) = &verification.divergence {
        bail!("the two replays diverge: {divergence}");
    }
    Ok(())
}

/// Read a file no larger than `limit` bytes.
#[cfg(feature = "sandbox")]
fn read_bounded(path: &std::path::Path, limit: u64, what: &str) -> Result<Vec<u8>> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("read {what} {}", path.display()))?;
    anyhow::ensure!(metadata.len() <= limit, "{what} exceeds {limit} bytes");
    std::fs::read(path).with_context(|| format!("read {what} {}", path.display()))
}

/// Drive a module through the calls `id world measure` reports on and return
/// the JSON report.
///
/// # Errors
///
/// A module that fails to compile, a malformed `--input`, or a call that traps.
#[cfg(feature = "sandbox")]
fn measure_module(
    wasm: &[u8],
    seed: u64,
    inputs: &[String],
    viewport: Option<(u16, u16)>,
) -> Result<serde_json::Value> {
    use crate::sandbox::{GuestCall, Sandbox, SandboxLimits};

    let events = inputs
        .iter()
        .map(|hex| {
            crate::world_session::decode_hex(hex)
                .with_context(|| format!("`{hex}` is not an even-length hex string"))
        })
        .collect::<Result<Vec<_>>>()?;
    let limits = SandboxLimits::default();
    let sandbox = Sandbox::compile(wasm, limits)?;
    let mut world = sandbox.instantiate(seed)?;
    world.view()?;
    if let Some((width, height)) = viewport {
        world.layout_at(width, height)?;
    }
    world.records()?;
    world.wants()?;
    world.snapshot()?;
    for event in &events {
        world.update(1, event)?;
        world.view()?;
    }

    let mut calls = serde_json::Map::new();
    let mut worst = (GuestCall::Init, 0);
    for (call, stats) in world.metrics().iter() {
        if stats.calls == 0 {
            continue;
        }
        if stats.fuel_max > worst.1 {
            worst = (call, stats.fuel_max);
        }
        calls.insert(
            call.name().to_owned(),
            serde_json::json!({
                "calls": stats.calls,
                "fuel_total": stats.fuel_total,
                "fuel_max": stats.fuel_max,
            }),
        );
    }
    #[allow(clippy::cast_precision_loss)]
    let worst_percent = worst.1 as f64 * 100.0 / limits.fuel as f64;
    Ok(serde_json::json!({
        "module_bytes": wasm.len(),
        "fuel_limit": limits.fuel,
        "calls": calls,
        "worst_call": worst.0.name(),
        "worst_call_percent_of_limit": (worst_percent * 100.0).round() / 100.0,
    }))
}

/// Scaffold a Roc world app in `dir`.
///
/// The directory must be absent or empty. Writes `main.roc`, `build.sh`
/// (executable), and `README.md`, and prints the directory path.
///
/// # Errors
///
/// A bad name, a non-empty directory, or a filesystem failure.
fn scaffold_world(dir: &std::path::Path, name: &str) -> Result<()> {
    anyhow::ensure!(
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "the app name may contain letters, digits, '-' and '_'"
    );
    if dir.exists() {
        anyhow::ensure!(
            std::fs::read_dir(dir)?.next().is_none(),
            "{} is not empty",
            dir.display()
        );
    } else {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(dir.join("main.roc"), template_main(name))
        .with_context(|| format!("write {}", dir.join("main.roc").display()))?;
    std::fs::write(dir.join("README.md"), template_readme(name))
        .with_context(|| format!("write {}", dir.join("README.md").display()))?;
    let build = dir.join("build.sh");
    std::fs::write(&build, template_build(name))
        .with_context(|| format!("write {}", build.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&build, std::fs::Permissions::from_mode(0o755))?;
    }
    println!("{}", dir.display());
    Ok(())
}

fn template_main(name: &str) -> String {
    format!(
        r#"app [program] {{ pf: platform "../roc-world/platform.roc" }}

import pf.Screen

# {name}: a scaffolded world app. Replace the model, update, and view with your
# own. Build with ./build.sh, then install with `id world install` or compile the
# source with `id world compile`.

Model : {{ count : U64 }}

program = {{ init, update, view, layout, records, wants, snapshot, restore }}

init : U64 -> Model
init = |_seed| {{ count: 0 }}

update : Model, U64, Str -> Model
update = |model, _pid, _ev| {{ count: model.count + 1 }}

view : Model -> Str
view = |model| {{
    screen = Screen.boxed(Screen.new(24, 3), 0, 0, 24, 3, "{name}")
    drawn = Screen.put_text(screen, 2, 1, "count=${{model.count.to_str()}}", {{ fg: 2, bg: 0, bold: Bool.True }})
    Screen.render_json(drawn)
}}

## Render the same view at any viewport.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |model| "{{\"count\":${{model.count.to_str()}}}}"

wants : Model -> Str
wants = |_model| "{{}}"

snapshot : Model -> Str
snapshot = |model| model.count.to_str()

restore : Str -> Model
restore = |text| match U64.from_str(text) {{
    Ok(count) => {{ count: count }}
    Err(_) => {{ count: 0 }}
}}
"#
    )
}

fn template_readme(name: &str) -> String {
    format!(
        r#"# {name}

A world app scaffolded with `id world new`.

## Build

```bash
./build.sh
```

The script expects a checkout of the world platform at `../roc-world` and
produces `{name}.wasm`, validated import-free.

## Run

```bash
id serve --data-dir ./data --world --world-admin-token "$ADMIN" \
  --world-module "$PWD/{name}.wasm" --world-ssh-port 2223 --web --port 8787
```

## Next steps

- Edit `update` to react to input events and `view` to draw a portable screen.
- Ask for host capabilities through `wants`; grant them with `id world caps`.
- Keep `snapshot`/`restore` faithful so the host can compact the journal.
"#
    )
}

fn template_build(name: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${{BASH_SOURCE[0]}}")"
ROC="${{ROC:-roc}}"

if [ -z "${{WASM_TOOLS:-}}" ]; then
  if command -v wasm-tools >/dev/null 2>&1; then
    WASM_TOOLS="wasm-tools"
  else
    STORE_TOOL=$(ls -d /nix/store/*-wasm-tools-*/bin/wasm-tools 2>/dev/null | head -n1 || true)
    if [ -n "$STORE_TOOL" ] && [ -x "$STORE_TOOL" ]; then
      WASM_TOOLS="$STORE_TOOL"
    else
      WASM_TOOLS="nix run nixpkgs#wasm-tools --"
    fi
  fi
fi

(cd ../roc-world && ./build-host.sh >/dev/null)
"$ROC" build main.roc --target=wasm32 --debug --wasm-memory=8454144 --output={name}.wasm
$WASM_TOOLS validate {name}.wasm
echo "Built {name}.wasm (import-free Roc world module)"
"#
    )
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

        let (key, blobs, docs) = if let Some(dir) = dir {
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
        } else {
            let blobs: iroh_blobs::api::Store = MemStore::new().into();
            (
                load_or_create_keypair(CLIENT_KEY_FILE).await?,
                blobs,
                iroh_docs::protocol::Docs::memory(),
            )
        };
        let endpoint = if no_relay {
            Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .secret_key(key)
                .bind()
                .await?
        } else {
            crate::connectivity::public_builder(crate::connectivity::RelayPolicy::from_no_relay(
                no_relay,
            ))
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
/// Notes for the user go to stderr, so stdout stays a stream of frames.
///
/// The transport is any byte stream carrying the length-prefixed envelope
/// (an iroh stream or an SSH exec channel); `first` is the join frame.
async fn pump<R, W>(
    mut recv: R,
    mut send: W,
    first: &str,
    command_mode: bool,
    capability: &str,
) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin,
{
    write_frame_to(&mut send, first.as_bytes()).await?;
    let mut reader = tokio::spawn(async move {
        while let Some(body) = read_frame_from(&mut recv).await? {
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
                    let frame = if command_mode {
                        match pump_command(line, capability) {
                            PumpLine::Send(frame) => frame,
                            PumpLine::Say(text) => {
                                eprintln!("{text}");
                                continue;
                            }
                            PumpLine::Quit => break Ok(()),
                        }
                    } else {
                        match line.strip_prefix("/input ") {
                            Some(hex) => serde_json::json!({"type": "input", "data_hex": hex.trim()}),
                            None => serde_json::json!({"type": "chat", "text": line}),
                        }
                    };
                    if let Err(e) = write_frame_to(&mut send, frame.to_string().as_bytes()).await {
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
    reader.abort();
    result
}

/// Check a transcript against the shell contract, returning a summary.
///
/// # Errors
///
/// Fails on an unreadable transcript, or with every violation listed.
fn check_contract(text: &str) -> Result<String> {
    let contract = crate::shell_contract::ShellContract::embedded().map_err(anyhow::Error::msg)?;
    let violations = crate::shell_contract::check_transcript(contract, text)?;
    let frames = text.lines().filter(|line| !line.trim().is_empty()).count();
    if violations.is_empty() {
        return Ok(format!(
            "{frames} frames conform to shell contract v{}.{}",
            contract.version, contract.revision
        ));
    }
    let listed: Vec<String> = violations.iter().map(ToString::to_string).collect();
    bail!(
        "{} of {frames} frames violate shell contract v{}.{}:\n{}",
        violations.len(),
        contract.version,
        contract.revision,
        listed.join("\n")
    )
}

/// What one stdin line does in the `id world join --command-mode` pump.
#[derive(Debug, PartialEq, Eq)]
enum PumpLine {
    /// Send this session frame.
    Send(serde_json::Value),
    /// Answer the user locally; nothing is sent.
    Say(String),
    /// End the session.
    Quit,
}

const PUMP_HELP: &str = "Commands: a line is chat; /input <hex>, /records [prefix], /stats, /caps, /logs, /help, /quit. Server frames print to stdout as JSON lines.";

/// Map one line of the shared grammar ([`crate::world_line`]) to what the
/// pump does. Every frame it sends is in the shell contract's joined phase;
/// the commands that need the host's own store (`/documents`) or that the
/// pump already does (`/logs`) are answered locally.
fn pump_command(line: &str, capability: &str) -> PumpLine {
    use crate::world_line::{WorldLine, parse_line};
    let Some(action) = parse_line(line) else {
        return PumpLine::Quit;
    };
    PumpLine::Send(match action {
        WorldLine::Chat(text) => serde_json::json!({"type": "chat", "text": text}),
        WorldLine::Input(hex) => serde_json::json!({"type": "input", "data_hex": hex}),
        WorldLine::Records { prefix } => serde_json::json!({
            "type": "records", "capability": capability, "prefix": prefix, "after": null
        }),
        WorldLine::Stats => serde_json::json!({"type": "info", "capability": capability}),
        WorldLine::Caps => serde_json::json!({"type": "caps", "capability": capability}),
        WorldLine::Documents { .. } | WorldLine::Document(_) => {
            return PumpLine::Say(
                "documents are read from the host's own store over SSH (`ssh cmd:<world>@host`); `id world join` has no document reader".to_owned(),
            );
        }
        WorldLine::Logs { .. } => {
            return PumpLine::Say(
                "events already print live here as JSON lines; use `id world logs` for a bounded tail".to_owned(),
            );
        }
        WorldLine::Help => return PumpLine::Say(PUMP_HELP.to_owned()),
        WorldLine::Quit => return PumpLine::Quit,
        WorldLine::Message(message) => return PumpLine::Say(message),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{PumpLine, check_contract, pump_command, scaffold_world, step_is_applied};

    /// #818, updated for #807: every step kind is applied by `apply_steps`
    /// over the wire — module install and schedules included — so every
    /// step counts toward the applied summary.
    #[test]
    fn every_step_kind_counts_as_applied() {
        use crate::world_manifest::ApplyStep;
        let steps = [
            ApplyStep::InstallModule {
                hash: Some("abc".to_owned()),
                source: None,
                seed: 0,
            },
            ApplyStep::SetGrants { grants: vec![] },
            ApplyStep::SetCeiling { ceiling: None },
            ApplyStep::SetProfile {
                profile: crate::world_limits::WorldExecutionProfile::default(),
            },
            ApplyStep::ScheduleAt {
                id: "round".to_owned(),
                at_ms: 5,
            },
            ApplyStep::CancelSchedule {
                id: "round".to_owned(),
            },
        ];
        let applied = steps.iter().filter(|step| step_is_applied(step)).count();
        assert_eq!(applied, steps.len());
    }

    /// The execution order is what makes a plan reachable (#807): a
    /// cancellation frees the schedule slot an addition needs, and the
    /// profile, grants and ceiling all land before the module step that
    /// compiles or installs under them.
    #[test]
    fn apply_steps_run_in_reachability_order() {
        use crate::world_manifest::ApplyStep;
        let steps = [
            ApplyStep::ScheduleAt {
                id: "new".to_owned(),
                at_ms: 5,
            },
            ApplyStep::InstallModule {
                hash: None,
                source: Some(std::path::PathBuf::from("main.roc")),
                seed: 0,
            },
            ApplyStep::SetCeiling { ceiling: None },
            ApplyStep::SetGrants { grants: vec![] },
            ApplyStep::SetProfile {
                profile: crate::world_limits::WorldExecutionProfile::default(),
            },
            ApplyStep::CancelSchedule {
                id: "old".to_owned(),
            },
        ];
        let kinds: Vec<&str> = super::ordered_steps(&steps)
            .iter()
            .map(|step| step.kind())
            .collect();
        assert_eq!(
            kinds,
            [
                "cancel_schedule",
                "set_profile",
                "set_grants",
                "set_ceiling",
                "install_module",
                "schedule_at"
            ]
        );
    }

    /// A manifest `module_source` that names a directory compiles its
    /// `main.roc` plus the sibling `.roc` files (and nothing else); a plain
    /// file compiles alone, always sent as `main.roc`.
    #[test]
    fn compile_sources_read_a_directorys_main_and_siblings() {
        let dir = std::env::temp_dir().join(format!("id-apply-sources-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.roc"), "main").unwrap();
        std::fs::write(dir.join("Helper.roc"), "helper").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let sources = super::read_compile_sources(&dir).unwrap();
        assert_eq!(
            sources,
            vec![
                ("main.roc".to_owned(), "main".to_owned()),
                ("Helper.roc".to_owned(), "helper".to_owned()),
            ]
        );
        let single = super::read_compile_sources(&dir.join("main.roc")).unwrap();
        assert_eq!(single, vec![("main.roc".to_owned(), "main".to_owned())]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The shared line grammar maps the same line to the same action on the
    /// join pump as it does on the SSH command mode (both call
    /// [`crate::world_line::parse_line`]); this asserts the pump's mapping,
    /// and that every frame it sends is in the shell contract.
    #[test]
    fn the_pump_command_mode_maps_the_shared_grammar_to_contract_frames() {
        use crate::shell_contract::{Phase, ShellContract};
        let send = |line: &str| match pump_command(line, "cap") {
            PumpLine::Send(frame) => frame,
            other => panic!("{line}: {other:?}"),
        };
        assert_eq!(
            send("/stats"),
            serde_json::json!({"type": "info", "capability": "cap"})
        );
        assert_eq!(
            send("/caps"),
            serde_json::json!({"type": "caps", "capability": "cap"})
        );
        assert_eq!(
            send("/records x"),
            serde_json::json!({"type": "records", "capability": "cap", "prefix": "x", "after": null})
        );
        assert_eq!(
            send("/input 00aF"),
            serde_json::json!({"type": "input", "data_hex": "00af"})
        );
        assert_eq!(
            send("hello"),
            serde_json::json!({"type": "chat", "text": "hello"})
        );
        let contract = ShellContract::embedded().unwrap();
        for line in [
            "/stats",
            "/caps",
            "/records",
            "/records x",
            "/input 00",
            "hi",
        ] {
            contract
                .check_client_frame(&send(line), Phase::Joined)
                .unwrap_or_else(|error| panic!("{line}: {error}"));
        }
        for line in ["/logs", "/help", "/documents", "/document a.md", "/nope"] {
            assert!(
                matches!(pump_command(line, "cap"), PumpLine::Say(_)),
                "{line}"
            );
        }
        assert_eq!(pump_command("/quit", "cap"), PumpLine::Quit);
    }

    #[test]
    fn the_contract_check_reports_conformance_or_every_violation() {
        let good = [
            r#"{"from": "client", "frame": {"type": "join", "capability": "c"}}"#,
            r#"{"from": "server", "frame": {"type": "snapshot", "snapshot": {}}}"#,
            r#"{"from": "client", "frame": {"type": "resize", "width": 80, "height": 24}}"#,
        ]
        .join("\n");
        assert_eq!(
            check_contract(&good).unwrap(),
            "3 frames conform to shell contract v1.3"
        );
        let bad = [
            r#"{"from": "client", "frame": {"type": "chat", "text": "too early"}}"#,
            r#"{"from": "server", "frame": {"type": "teleport"}}"#,
        ]
        .join("\n");
        let error = check_contract(&bad).unwrap_err().to_string();
        assert!(error.starts_with("2 of 2 frames violate"), "{error}");
        assert!(error.contains("frame 0 (client)"), "{error}");
        assert!(error.contains("frame 1 (server)"), "{error}");
        assert!(check_contract("{").is_err());
    }

    #[test]
    fn scaffolding_writes_a_buildable_app_skeleton() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("my-world");
        scaffold_world(&target, "my-world").unwrap();
        let main = std::fs::read_to_string(target.join("main.roc")).unwrap();
        assert!(main.contains("app [program]"));
        assert!(
            main.contains(
                "program = { init, update, view, layout, records, wants, snapshot, restore }"
            ),
            "{main}"
        );
        assert!(
            main.contains("layout = |model, _width, _height| view(model)"),
            "{main}"
        );
        assert!(main.contains("Screen.render_json"), "{main}");
        let build = std::fs::read_to_string(target.join("build.sh")).unwrap();
        assert!(build.contains("my-world.wasm"), "{build}");
        assert!(target.join("README.md").is_file());

        // A non-empty directory and a path-like name are refused.
        assert!(scaffold_world(&target, "my-world").is_err());
        let bad = dir.path().join("bad");
        assert!(scaffold_world(&bad, "../escape").is_err());
    }

    #[cfg(feature = "sandbox")]
    #[test]
    fn measure_reports_fuel_per_call_kind_for_a_checked_in_module() {
        let wasm = include_bytes!("../../examples/roc-counter/counter.wasm");
        let report =
            super::measure_module(wasm, 7, &["696e63".to_owned()], Some((40, 20))).unwrap();
        assert_eq!(report["module_bytes"], wasm.len());
        assert_eq!(report["fuel_limit"], 10_000_000);
        let calls = &report["calls"];
        // The initial view plus one after the input.
        assert_eq!(calls["view"]["calls"], 2);
        for kind in ["init", "update", "layout", "records", "wants", "snapshot"] {
            assert_eq!(calls[kind]["calls"], 1, "{kind}: {report}");
            assert!(calls[kind]["fuel_max"].as_u64().unwrap() > 0, "{kind}");
        }
        assert!(calls.get("restore").is_none(), "{report}");
        // The worst call is layout, rendering the pf.Screen frame. Its
        // ceiling is the largest per-call budget FUEL_BUDGETS allows this
        // module (500_000 of the 10_000_000 limit), so the report must stay
        // under 5 percent; before the Screen migration the bare-text view
        // stayed under 1.
        assert!(report["worst_call_percent_of_limit"].as_f64().unwrap() < 5.0);

        // Without a viewport there is no layout call; bad hex is refused
        // before the module runs.
        let report = super::measure_module(wasm, 7, &[], None).unwrap();
        assert!(report["calls"].get("layout").is_none());
        assert!(super::measure_module(wasm, 7, &["zz".to_owned()], None).is_err());
        assert!(super::measure_module(b"not wasm", 7, &[], None).is_err());
    }
}
