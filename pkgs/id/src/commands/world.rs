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
                Ok::<serde_json::Value, anyhow::Error>(serde_json::json!({
                    "world_id": info.world_id,
                    "active_module_hash": info.active_module_hash,
                    "active_module_provenance": info.active_module_provenance,
                    "current_sequence": info.current_sequence,
                    "caps": caps,
                    "profile": profile,
                }))
            }
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

program = {{ init, update, view, records, wants, snapshot, restore }}

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

#[cfg(test)]
mod tests {
    use super::scaffold_world;

    #[test]
    fn scaffolding_writes_a_buildable_app_skeleton() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("my-world");
        scaffold_world(&target, "my-world").unwrap();
        let main = std::fs::read_to_string(target.join("main.roc")).unwrap();
        assert!(main.contains("app [program]"));
        assert!(
            main.contains("program = { init, update, view, records, wants, snapshot, restore }"),
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
