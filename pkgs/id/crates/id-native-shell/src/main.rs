//! `id-native-shell` binary: the Bevy application target for F13.6.
//!
//! Step 2 (design #788) replaces step 1's `MinimalPlugins` placeholder
//! with Bevy's window/render/UI plugin set (`DefaultPlugins` over the
//! crate's selected features) and the shell's own render plugin
//! ([`id_native_shell::render::ShellRenderPlugin`]), which presents
//! the latest host view as a UI node tree, plus the scene renderer
//! ([`id_native_shell::scene::SceneRenderPlugin`]), which presents a
//! view's validated `scene/1` surface as 3D entities, plus the audio
//! renderer ([`id_native_shell::audio::AudioRenderPlugin`]), which
//! presents a view's validated `audio/1` surface as scheduled cue
//! entities, plus the widget renderer
//! ([`id_native_shell::widget::WidgetRenderPlugin`]), which presents
//! a view's validated `widget/1` surface as placed control entities.
//!
//! With `--node <id> --capability <cap>` the binary also runs the live
//! session: [`id_native_shell::run_session`] joins the world on a
//! background thread, and every host view it publishes is drained into
//! [`PresentedView`] by `drain_session_views`. Without a target the
//! window opens on an empty view and the announce line says so, rather
//! than pretending to be connected. Outbound frames travel the
//! session's channel (held in [`SessionBridge`]); wiring Bevy keyboard
//! / clipboard capture to it is presentation work on top of the
//! already-tested [`id_native_shell::events`] mapping.

use std::net::SocketAddr;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use bevy::prelude::{App, DefaultPlugins, Res, ResMut, Resource, Startup, Update};
use id_native_shell::audio::AudioRenderPlugin;
use id_native_shell::render::{PresentedForm, PresentedView, ShellRenderPlugin};
use id_native_shell::scene::SceneRenderPlugin;
use id_native_shell::widget::WidgetRenderPlugin;
use id_native_shell::{
    CONTRACT_NAME, CONTRACT_VERSION, LocalBounds, ProviderAdapter, SessionView,
    run_session_with_providers,
};
use iroh::endpoint::{RelayMode, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, TransportAddr};
use serde_json::Value;
use tokio::sync::mpsc;

/// Command-line target for one live session (the same connection
/// parameters `id world join` takes).
#[derive(Debug, Default)]
struct SessionArgs {
    node: Option<String>,
    capability: Option<String>,
    world: Option<String>,
    addrs: Vec<SocketAddr>,
    no_relay: bool,
}

/// Parse the session arguments; `--help` and unknown flags are errors
/// with the usage text, so a mistyped launch fails loudly instead of
/// opening a disconnected window.
fn parse_args() -> Result<SessionArgs> {
    const USAGE: &str = "usage: id-native-shell [--node <node-id> --capability <cap>] \
                         [--world <name>] [--addr <ip:port>]... [--no-relay]";
    let mut args = SessionArgs::default();
    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = |name: &str| -> Result<String> {
            iter.next()
                .with_context(|| format!("{name} needs a value\n{USAGE}"))
        };
        match flag.as_str() {
            "--node" => args.node = Some(value("--node")?),
            "--capability" => args.capability = Some(value("--capability")?),
            "--world" => args.world = Some(value("--world")?),
            "--addr" => {
                let text = value("--addr")?;
                args.addrs.push(
                    text.parse()
                        .with_context(|| format!("parse --addr {text}\n{USAGE}"))?,
                );
            }
            "--no-relay" => args.no_relay = true,
            other => bail!("unknown argument `{other}`\n{USAGE}"),
        }
    }
    if args.node.is_some() != args.capability.is_some() {
        bail!("--node and --capability must be given together\n{USAGE}");
    }
    Ok(args)
}

/// Channels between the session thread and the Bevy app: host views
/// flow in, outbound contract frames flow out. Holding the outbound
/// sender keeps the session open for the app's lifetime; dropping the
/// bridge (app exit) closes the channel and ends the session.
#[derive(Resource)]
struct SessionBridge {
    views: Mutex<mpsc::UnboundedReceiver<SessionView>>,
    _outbound: mpsc::Sender<Value>,
}

/// Drain every view the session has published into the presented view.
///
/// #8938: each drained view is also folded through the form presenter,
/// so a trusted presented form renders widgets and any other view clears
/// them.
fn drain_session_views(
    mut presented: ResMut<'_, PresentedView>,
    mut form: ResMut<'_, PresentedForm>,
    bridge: Res<'_, SessionBridge>,
) {
    let Ok(mut views) = bridge.views.lock() else {
        return;
    };
    while let Ok(view) = views.try_recv() {
        form.apply_view(&view);
        presented.0 = Some(view);
    }
}

/// Dial the host and run the session on a background thread with its
/// own Tokio runtime (the Bevy main loop owns this thread). Session
/// failures are reported on stderr; the window stays up on the last
/// presented view.
fn spawn_session(args: &SessionArgs) -> Result<SessionBridge> {
    let node = args.node.clone().context("session node")?;
    let capability = args.capability.clone().context("session capability")?;
    let world = args.world.clone();
    let addrs = args.addrs.clone();
    let no_relay = args.no_relay;
    let (views_tx, views_rx) = mpsc::unbounded_channel::<SessionView>();
    let (outbound_tx, outbound_rx) = mpsc::channel::<Value>(32);
    let provider_outbound = outbound_tx.clone();
    std::thread::Builder::new()
        .name("id-native-shell-session".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("id-native-shell: cannot start session runtime: {error:#}");
                    return;
                }
            };
            runtime.block_on(async move {
                match connect(&node, &addrs, no_relay).await {
                    Ok((endpoint, client)) => {
                        // F13.7 step 4: the local provider adapter. The
                        // shipped shell registers no executor and holds
                        // no grants, so nothing is offered and any
                        // provider_request the host sends is refused by
                        // the local check and recorded in the local
                        // audit log (printed to stderr, the shell's
                        // user-visible surface). Grants/executors are
                        // registered on this adapter by the local
                        // opt-in surface; the adapter alone decides
                        // what executes.
                        let mut adapter = ProviderAdapter::new();
                        // F13.7 step 5: the one local opt-in the
                        // shipped shell has. Setting
                        // ID_SHELL_FILE_READ_ROOT to a directory is
                        // the local user's deliberate grant of
                        // client.file.read for this session's world,
                        // under the design's default local bounds —
                        // and it is simultaneously the durability
                        // disclosure acknowledgment: bytes read under
                        // this root and returned to the world become
                        // journaled world data, visible to the
                        // host/operator and anyone with journal
                        // access. Unset (the default), nothing is
                        // registered, offered, or executed.
                        if let Ok(root) = std::env::var("ID_SHELL_FILE_READ_ROOT") {
                            let world_name = world.as_deref().unwrap_or_default();
                            match adapter.grant_file_read(
                                world_name,
                                root,
                                LocalBounds::DESIGN_DEFAULT,
                                true,
                            ) {
                                Ok(()) => eprintln!(
                                    "id-native-shell: client.file.read enabled for world {world_name} under ID_SHELL_FILE_READ_ROOT; returned bytes become journaled world data"
                                ),
                                Err(code) => eprintln!(
                                    "id-native-shell: client.file.read opt-in refused: {code}"
                                ),
                            }
                        }
                        let provider_offer =
                            adapter.offer_frame(world.as_deref().unwrap_or_default());
                        let (provider_tx, mut provider_rx) =
                            mpsc::unbounded_channel::<Value>();
                        let provider_world = world.clone().unwrap_or_default();
                        let provider_task = tokio::spawn(async move {
                            while let Some(request) = provider_rx.recv().await {
                                let audited = adapter.audit_entries().len();
                                let now_ms = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map_or(0, |elapsed| {
                                        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
                                    });
                                let reply =
                                    adapter.handle_request(&provider_world, &request, now_ms);
                                for entry in &adapter.audit_entries()[audited..] {
                                    eprintln!(
                                        "id-native-shell: local provider audit: world={}                                          provider={} nonce={} outcome={} reason={}",
                                        entry.world,
                                        entry.provider,
                                        entry.nonce,
                                        entry.outcome,
                                        entry.reason.as_deref().unwrap_or("-"),
                                    );
                                }
                                if provider_outbound.send(reply).await.is_err() {
                                    break;
                                }
                            }
                        });
                        let result = run_session_with_providers(
                            client,
                            world.as_deref(),
                            &capability,
                            None,
                            outbound_rx,
                            views_tx,
                            provider_tx,
                            provider_offer,
                        )
                        .await;
                        provider_task.abort();
                        endpoint.close().await;
                        if let Err(error) = result {
                            eprintln!("id-native-shell: session ended with error: {error:#}");
                        }
                    }
                    Err(error) => {
                        eprintln!("id-native-shell: cannot connect to world host: {error:#}");
                    }
                }
            });
        })
        .context("spawn session thread")?;
    Ok(SessionBridge {
        views: Mutex::new(views_rx),
        _outbound: outbound_tx,
    })
}

/// Dial a world host, mirroring the CLI's `world join` connection:
/// the shared client keypair, a public endpoint when no direct
/// addresses are given, a relay-free loopback endpoint otherwise.
async fn connect(
    node: &str,
    addrs: &[SocketAddr],
    no_relay: bool,
) -> Result<(Endpoint, id::world_net::WorldClient)> {
    if node.len() != 64 || !node.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("`{node}` is not a node ID (expected 64 hex characters)");
    }
    let id: EndpointId = node.parse().context("parse node ID")?;
    let key = id::load_or_create_keypair(id::CLIENT_KEY_FILE)
        .await
        .context("load client keypair")?;
    let endpoint = if addrs.is_empty() {
        id::connectivity::public_builder(id::connectivity::RelayPolicy::from_no_relay(no_relay))
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
    let client = id::world_net::WorldClient::connect(&endpoint, addr)
        .await
        .context("connect to world host")?;
    Ok((endpoint, client))
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
    };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        .add_plugins(ShellRenderPlugin)
        .add_plugins(SceneRenderPlugin)
        .add_plugins(AudioRenderPlugin)
        .add_plugins(WidgetRenderPlugin)
        .add_systems(Startup, announce);
    if args.node.is_some() {
        match spawn_session(&args) {
            Ok(bridge) => {
                app.insert_resource(bridge)
                    .add_systems(Update, drain_session_views);
            }
            Err(error) => {
                eprintln!("id-native-shell: cannot start session: {error:#}");
                std::process::exit(2);
            }
        }
    }
    app.run();
}

/// Announce the shell and the contract it speaks, once, at startup.
fn announce(bridge: Option<Res<'_, SessionBridge>>) {
    let session_line = if bridge.is_some() {
        "live WorldClient session running; host views feed the renderer"
    } else {
        "no --node/--capability given; window opens without a session"
    };
    println!(
        "id-native-shell: F13.6 native reference shell; {CONTRACT_NAME} contract \
         v{CONTRACT_VERSION}; window/render wiring is live (design #788 step 2); {session_line}"
    );
}
