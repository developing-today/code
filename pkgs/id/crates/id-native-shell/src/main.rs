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
//! With `--node <id> --capability <cap>` and `--world <name>` the
//! binary also runs the live session: [`id_native_shell::run_session`]
//! joins the world on a background thread, and every host view it
//! publishes is drained into [`PresentedView`] by
//! `drain_session_views`. Without `--world` the binary opens the
//! world directory instead (#15022, design #5782 §3): a probe
//! thread reads the host's public directory at start, on an
//! interval and on the refresh key, the
//! [`id_native_shell::directory::DirectoryPlugin`] presents it, and
//! each world the user enters gets its own session bridge, drained
//! by `drive_directory` — leaving one world never tears down
//! another's session. Without a target the window opens on an
//! empty view and the announce line says so, rather than
//! pretending to be connected. Outbound frames travel the
//! session's channel (held in [`SessionBridge`]): key presses in the
//! window reach the foreground world as the terminal's own `input`
//! frames, mapped by [`id_native_shell::events::key_input_bytes`]
//! (in directory mode Esc stays the way back to the list).
//! Clipboard paste and window resize are later slices on the same
//! tested [`id_native_shell::events`] mapping.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use bevy::clipboard::Clipboard;
use bevy::input::ButtonInput;
use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::{
    App, DefaultPlugins, IntoScheduleConfigs, KeyCode, MessageReader, Res, ResMut, Resource,
    Startup, Update,
};
use id_native_shell::audio::AudioRenderPlugin;
use id_native_shell::directory::{
    DIRECTORY_REFRESH_INTERVAL, DirectoryApp, DirectoryEffect, DirectoryFocus,
    DirectoryInputSystems, DirectoryMarks, DirectoryPlugin,
};
use id_native_shell::events::{
    KeyModifiers, NativeEvent, is_paste_chord, key_input_bytes, native_event_frame, paste_event,
};
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
/// parameters `id world join` takes). Also the directory mode's
/// connection target, so it is kept as a resource there.
#[derive(Debug, Default, Resource)]
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
    outbound: mpsc::Sender<Value>,
}

/// The modifier keys held right now, for the terminal key mapping.
fn held_modifiers(held: Option<&ButtonInput<KeyCode>>) -> KeyModifiers {
    held.map_or_else(KeyModifiers::default, |held| KeyModifiers {
        ctrl: held.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]),
        alt: held.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]),
        shift: held.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
    })
}

/// Queue one outbound frame without blocking the Bevy frame: a full
/// queue drops it with a notice naming `what`, and an ended session
/// drops it silently.
fn try_send_frame(bridge: &SessionBridge, frame: Value, what: &str) {
    if let Err(mpsc::error::TrySendError::Full(_)) = bridge.outbound.try_send(frame) {
        eprintln!("id-native-shell: {what} dropped: the session is not keeping up");
    }
}

/// Paste the system clipboard into a session as one contract `paste`
/// frame (#15022). An empty clipboard sends nothing. A clipboard the
/// platform cannot read, or text over the contract bound, is reported
/// on stderr and sends nothing: a paste is never split or truncated.
fn send_paste(bridge: &SessionBridge, clipboard: Option<&mut Clipboard>) {
    let Some(clipboard) = clipboard else {
        eprintln!("id-native-shell: paste not sent: no clipboard is available");
        return;
    };
    let text = match clipboard.fetch_text().poll_result() {
        Some(Ok(text)) => text,
        Some(Err(error)) => {
            eprintln!("id-native-shell: paste not sent: clipboard read failed: {error}");
            return;
        }
        None => {
            eprintln!("id-native-shell: paste not sent: the clipboard read did not complete");
            return;
        }
    };
    let Some(event) = paste_event(&text) else {
        return;
    };
    match native_event_frame(&event) {
        Ok(frame) => try_send_frame(bridge, frame, "paste"),
        Err(error) => eprintln!("id-native-shell: paste not sent: {error}"),
    }
}

/// Send one key press to a session: the paste chord pastes the
/// clipboard, any other key goes as terminal input.
fn send_key_or_paste(
    bridge: &SessionBridge,
    press: &KeyboardInput,
    modifiers: KeyModifiers,
    clipboard: Option<&mut Clipboard>,
) {
    if is_paste_chord(&press.logical_key, modifiers) {
        send_paste(bridge, clipboard);
    } else {
        send_key(bridge, press, modifiers);
    }
}

/// Send one key press to a session as the contract `input` frame,
/// if the terminal mapping encodes it. Never blocks the frame: a
/// session whose outbound queue is full drops the key and says so
/// on stderr, and a session that has ended drops it silently.
fn send_key(bridge: &SessionBridge, press: &KeyboardInput, modifiers: KeyModifiers) {
    let Some(bytes) = key_input_bytes(&press.logical_key, press.text.as_deref(), modifiers) else {
        return;
    };
    let frame = match native_event_frame(&NativeEvent::KeyInput(bytes)) {
        Ok(frame) => frame,
        Err(error) => {
            eprintln!("id-native-shell: key not sent: {error}");
            return;
        }
    };
    try_send_frame(bridge, frame, "key");
}

/// Direct-join mode: every key press in the window goes to the one
/// session as terminal input, and the paste chord pastes the
/// clipboard into it.
#[allow(clippy::needless_pass_by_value)]
fn forward_session_keys(
    mut keys: MessageReader<'_, '_, KeyboardInput>,
    held: Option<Res<'_, ButtonInput<KeyCode>>>,
    bridge: Res<'_, SessionBridge>,
    mut clipboard: Option<ResMut<'_, Clipboard>>,
) {
    let modifiers = held_modifiers(held.as_deref());
    for press in keys.read() {
        if press.state == ButtonState::Pressed {
            send_key_or_paste(&bridge, press, modifiers, clipboard.as_deref_mut());
        }
    }
}

/// Directory mode: key presses go to the foreground world's session
/// while a session has focus, except Esc, which returns to the
/// list. Runs before the directory's own key handling, so a key is
/// judged by the focus it was pressed in: the Enter that opens a
/// world is not sent to it, and the Esc that leaves one is not
/// either. The paste chord pastes the clipboard into the foreground
/// world only, never into the list.
#[allow(clippy::needless_pass_by_value)]
fn forward_directory_session_keys(
    mut keys: MessageReader<'_, '_, KeyboardInput>,
    held: Option<Res<'_, ButtonInput<KeyCode>>>,
    directory: Res<'_, DirectoryApp>,
    sessions: Res<'_, WorldSessions>,
    mut clipboard: Option<ResMut<'_, Clipboard>>,
) {
    let modifiers = held_modifiers(held.as_deref());
    for press in keys.read() {
        if press.state != ButtonState::Pressed {
            continue;
        }
        let Some(world) = directory.session_input_target(press.key_code == KeyCode::Escape) else {
            continue;
        };
        if let Some(session) = sessions.0.get(world) {
            send_key_or_paste(&session.bridge, press, modifiers, clipboard.as_deref_mut());
        }
    }
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

/// File the directory marks persist to, in the working directory
/// beside the client key file: pins and recents are this client's
/// own state, never host data.
const DIRECTORY_MARKS_FILE: &str = "directory-marks.json";

/// Load the persisted directory marks, if any. A missing or
/// damaged file reads as no marks (the model's parser is
/// forgiving by design): marks must never stop the app opening.
fn load_marks() -> DirectoryMarks {
    std::fs::read_to_string(DIRECTORY_MARKS_FILE)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .map_or_else(DirectoryMarks::default, |value| {
            DirectoryMarks::from_json(&value)
        })
}

/// Persist the directory marks after a change (best effort: a
/// failed save is reported, never fatal).
fn save_marks(marks: &DirectoryMarks) {
    if let Ok(text) = serde_json::to_string(&marks.to_json()) {
        if let Err(error) = std::fs::write(DIRECTORY_MARKS_FILE, text) {
            eprintln!("id-native-shell: cannot save directory marks: {error}");
        }
    }
}

/// The directory probe's hand-off to the app: each host directory
/// reply the probe thread reads, plus the channel the app uses to
/// ask for a re-read now (the refresh key).
#[derive(Resource)]
struct DirectoryBridge {
    listings: Mutex<mpsc::UnboundedReceiver<(String, Vec<(String, bool, usize)>)>>,
    refresh: mpsc::UnboundedSender<()>,
}

/// Read the host's public world directory (the existing
/// discovery API) and hand each reply to the app: once at start,
/// then every `DIRECTORY_REFRESH_INTERVAL` and whenever the app
/// asks through the refresh channel (#15022). Each read uses its
/// own short-lived connection, so the probe owns no session: each
/// world the user opens gets its own session connection, and a
/// read failing or the host going away never disturbs a live
/// session. The thread ends when the app drops its side.
fn spawn_directory_probe(args: &SessionArgs) -> Result<DirectoryBridge> {
    let node = args.node.clone().context("session node")?;
    let addrs = args.addrs.clone();
    let no_relay = args.no_relay;
    let (listings_tx, listings_rx) = mpsc::unbounded_channel();
    let (refresh_tx, mut refresh_rx) = mpsc::unbounded_channel::<()>();
    std::thread::Builder::new()
        .name("id-native-shell-directory".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Runtime::new() {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("id-native-shell: cannot start directory runtime: {error:#}");
                    return;
                }
            };
            runtime.block_on(async move {
                let mut interval = tokio::time::interval(DIRECTORY_REFRESH_INTERVAL);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        _ = interval.tick() => {}
                        request = refresh_rx.recv() => {
                            if request.is_none() {
                                break;
                            }
                            interval.reset();
                        }
                    }
                    if listings_tx.is_closed() {
                        break;
                    }
                    match connect(&node, &addrs, no_relay).await {
                        Ok((endpoint, mut client)) => {
                            match client.directory().await {
                                Ok(listing) => {
                                    if listings_tx.send(listing).is_err() {
                                        endpoint.close().await;
                                        break;
                                    }
                                }
                                Err(error) => {
                                    eprintln!(
                                        "id-native-shell: cannot read world directory: {error:#}"
                                    );
                                }
                            }
                            endpoint.close().await;
                        }
                        Err(error) => {
                            eprintln!("id-native-shell: cannot connect to world host: {error:#}");
                        }
                    }
                    // Requests that arrived during the read are
                    // answered by the read that just finished.
                    while refresh_rx.try_recv().is_ok() {}
                }
            });
        })
        .context("spawn directory thread")?;
    Ok(DirectoryBridge {
        listings: Mutex::new(listings_rx),
        refresh: refresh_tx,
    })
}

/// One live per-world session in directory mode: its bridge plus
/// the latest view it published, kept so re-foregrounding a world
/// shows its current state rather than a blank.
struct WorldSession {
    bridge: SessionBridge,
    last_view: Option<SessionView>,
}

/// Directory mode's session bridges, one per opened world
/// (design #5782 §3: sessions coexist; closing a world drops
/// only its own bridge).
#[derive(Resource, Default)]
struct WorldSessions(HashMap<String, WorldSession>);

/// Directory mode's per-frame work: fold the probe's listing
/// into the directory, perform the effects the model queued
/// (open or close per-world session bridges, persist marks), and
/// drain every live bridge — publishing only the foreground
/// world's views, so background sessions stay live but
/// off-screen.
#[allow(clippy::needless_pass_by_value)]
fn drive_directory(
    mut directory: ResMut<'_, DirectoryApp>,
    mut presented: ResMut<'_, PresentedView>,
    mut form: ResMut<'_, PresentedForm>,
    probe: Res<'_, DirectoryBridge>,
    mut sessions: ResMut<'_, WorldSessions>,
    args: Res<'_, SessionArgs>,
) {
    if let Ok(mut listings) = probe.listings.lock() {
        while let Ok((default, listing)) = listings.try_recv() {
            directory.set_listing(&default, &listing);
        }
    }
    for effect in directory.take_effects() {
        match effect {
            DirectoryEffect::EnterWorld(name) => {
                if !sessions.0.contains_key(&name) {
                    match spawn_session(&args, Some(name.clone())) {
                        Ok(bridge) => {
                            sessions.0.insert(
                                name,
                                WorldSession {
                                    bridge,
                                    last_view: None,
                                },
                            );
                        }
                        Err(error) => {
                            eprintln!(
                                "id-native-shell: cannot start session for {name}: {error:#}"
                            );
                        }
                    }
                }
            }
            DirectoryEffect::CloseWorld(name) => {
                sessions.0.remove(&name);
            }
            DirectoryEffect::MarksChanged => save_marks(&directory.marks),
            DirectoryEffect::RefreshDirectory => {
                let _ = probe.refresh.send(());
            }
        }
    }
    let foreground = directory.registry.foreground().map(str::to_owned);
    let in_session = directory.focus == DirectoryFocus::Session;
    for (name, session) in &mut sessions.0 {
        let drained: Vec<SessionView> = {
            let Ok(mut views) = session.bridge.views.lock() else {
                continue;
            };
            let mut drained = Vec::new();
            while let Ok(view) = views.try_recv() {
                drained.push(view);
            }
            drained
        };
        if let Some(last) = drained.last() {
            session.last_view = Some(last.clone());
        }
        if in_session && foreground.as_deref() == Some(name.as_str()) {
            for view in &drained {
                form.apply_view(view);
            }
            if let Some(last) = drained.last() {
                presented.0 = Some(last.clone());
            }
        }
    }
}

/// Dial the host and run the session on a background thread with its
/// own Tokio runtime (the Bevy main loop owns this thread). Session
/// failures are reported on stderr; the window stays up on the last
/// presented view.
fn spawn_session(args: &SessionArgs, world: Option<String>) -> Result<SessionBridge> {
    let node = args.node.clone().context("session node")?;
    let capability = args.capability.clone().context("session capability")?;
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
        outbound: outbound_tx,
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
    if args.node.is_some() && args.world.is_some() {
        // Direct join: a named world opens its session at once,
        // the pre-directory behaviour.
        match spawn_session(&args, args.world.clone()) {
            Ok(bridge) => {
                app.insert_resource(bridge)
                    .add_systems(Update, (drain_session_views, forward_session_keys));
            }
            Err(error) => {
                eprintln!("id-native-shell: cannot start session: {error:#}");
                std::process::exit(2);
            }
        }
    } else if args.node.is_some() {
        // Directory mode (#15022, design #5782 §3): the app opens
        // on the host's world directory; each world the user
        // enters gets its own session bridge, and leaving one
        // never tears down the others.
        match spawn_directory_probe(&args) {
            Ok(probe) => {
                app.insert_resource(DirectoryApp::with_marks(load_marks()))
                    .insert_resource(probe)
                    .insert_resource(WorldSessions::default())
                    .insert_resource(args)
                    .add_plugins(DirectoryPlugin)
                    .add_systems(
                        Update,
                        (
                            forward_directory_session_keys.before(DirectoryInputSystems),
                            drive_directory,
                        ),
                    );
            }
            Err(error) => {
                eprintln!("id-native-shell: cannot start directory: {error:#}");
                std::process::exit(2);
            }
        }
    }
    app.run();
}

/// Announce the shell and the contract it speaks, once, at startup.
fn announce(bridge: Option<Res<'_, SessionBridge>>, directory: Option<Res<'_, DirectoryBridge>>) {
    let session_line = if bridge.is_some() {
        "live WorldClient session running; host views feed the renderer"
    } else if directory.is_some() {
        "world directory open; pick a world to enter its session"
    } else {
        "no --node/--capability given; window opens without a session"
    };
    println!(
        "id-native-shell: F13.6 native reference shell; {CONTRACT_NAME} contract \
         v{CONTRACT_VERSION}; window/render wiring is live (design #788 step 2); {session_line}"
    );
}
