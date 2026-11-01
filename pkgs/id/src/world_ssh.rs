//! SSH presentation for a hosted world.
//!
//! The SSH user names the world (empty for the default world) and the SSH
//! password is the join capability. Keystrokes become `input` frames and each
//! `view` frame repaints the terminal (incrementally for stable raw SSH screens),
//! so an ordinary `ssh` client can play any world that presents a view.
//!
//! ## Input modes
//!
//! Raw keystrokes are the default. Prefixing the username with `cmd:` enters
//! command (line) mode, where each line is parsed as a command instead of
//! forwarded as an input keystroke: `/help`, `/quit`, `/input <hex>`,
//! `/records [prefix]`, `/documents [prefix]` and `/document <name>` (see
//! [`world_command`]). Prefixing the username with `explore:` enters a
//! read-only explore session, the only mode that supports public-key login.
//! See the SSH input-modes decision record.
//!
//! ## Frame transport (F13.2)
//!
//! An `exec` request for [`WORLD_EXEC_COMMAND`] turns the channel into a
//! transport for the shell contract itself: the same length-prefixed JSON
//! frames as the iroh `/id-world/1` stream, byte for byte, so
//! `id world join --via ssh` reaches a world through jump hosts and networks
//! that only pass SSH. The SSH login grants nothing on this channel: as on
//! iroh, the capability or admin secret inside the frames is the only
//! authority, and the frames route themselves with their `world` field.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_lite::StreamExt;
use russh::{
    Channel, ChannelId, ChannelOpenFailure, MethodKind, MethodSet, Pty,
    keys::{Algorithm, PrivateKey, PublicKey, ssh_key::HashAlg, ssh_key::LineEnding},
    server::{Auth, ChannelOpenHandle, Config, Handle, Handler, Msg, Server, Session},
};
use serde_json::json;
use tokio::{
    net::TcpListener,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};

use crate::directory_auth::Caller;
use crate::directory_view::{
    DirectoryAction, DirectoryOutcome, HELP, Line, parse_line, render_text,
};
use crate::frames::{Decoded, FrameBuffer, encode_frame};
use crate::world::hex_encode;
use crate::world_hub::{ResolveError, WorldHub};
use crate::world_session::{Inbound, SessionIo, encode_hex, run_session};

/// File, beside the other server keys, that holds the SSH host key.
pub const SSH_HOST_KEY_FILE: &str = ".ssh-host-key";

const INPUT_QUEUE: usize = 256;
/// User-name prefix that opens the directory explorer for a world.
const EXPLORE_PREFIX: &str = "explore:";
/// User-name prefix that opens the line-buffered command shell for a world.
const COMMAND_PREFIX: &str = "cmd:";
const MAX_PACKET_BYTES: u32 = 32 * 1024;
const INACTIVITY_TIMEOUT: Duration = Duration::from_mins(10);
/// Concurrent SSH connections the server accepts; the rest are refused at auth.
const MAX_CONNECTIONS: usize = 64;
/// Pause before a refused shell closes, so guessing costs at least this per connection.
const REFUSED_SHELL_DELAY: Duration = Duration::from_secs(1);
const PROMPT: &str = "id> ";
/// Longest line the explorer keeps; the rest of a longer line is dropped.
const MAX_LINE: usize = 1024;
/// Enter the alternate screen buffer, so a game's repaints stay out of scrollback.
const ALT_SCREEN_ENTER: &str = "\x1b[?1049h";
/// Leave the alternate screen buffer and restore the screen the user came from.
const ALT_SCREEN_LEAVE: &str = "\x1b[?1049l";
/// Input bytes dropped before the session tells the viewer about it.
const INPUT_DROP_NOTICE_BYTES: usize = 1024;
/// ALPN for SSH-over-iroh tunnels (F13.3): each bidirectional stream is one
/// raw SSH connection forwarded to the host's own SSH server. The tunnel
/// carries bytes only; SSH authentication happens inside it, unchanged.
pub const SSH_TUNNEL_ALPN: &[u8] = b"/id-ssh-tunnel/1";
/// Concurrent tunnelled connections one iroh connection may open.
const MAX_TUNNEL_STREAMS: usize = 16;

/// Host side of SSH-over-iroh: forwards each tunnel stream to the world SSH
/// server on loopback. Iroh peer identity grants nothing here; the peer
/// still has to authenticate to SSH inside the tunnel.
#[derive(Clone, Debug)]
pub struct SshTunnelProtocol {
    port: Arc<std::sync::OnceLock<u16>>,
}

impl SshTunnelProtocol {
    /// Tunnel to an SSH server already listening on `port`.
    #[must_use]
    pub fn with_port(port: u16) -> Self {
        let cell = std::sync::OnceLock::new();
        let _ = cell.set(port);
        Self {
            port: Arc::new(cell),
        }
    }

    /// Tunnel whose SSH port is bound after the router starts; the returned
    /// cell must be set once the SSH listener exists.
    #[must_use]
    pub fn deferred() -> (Self, Arc<std::sync::OnceLock<u16>>) {
        let cell = Arc::new(std::sync::OnceLock::new());
        (
            Self {
                port: Arc::clone(&cell),
            },
            cell,
        )
    }
}

impl iroh::protocol::ProtocolHandler for SshTunnelProtocol {
    async fn accept(
        &self,
        conn: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let permits = Arc::new(Semaphore::new(MAX_TUNNEL_STREAMS));
        while let Ok((send, recv)) = conn.accept_bi().await {
            let Some(port) = self.port.get().copied() else {
                continue;
            };
            let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                continue;
            };
            tokio::spawn(async move {
                let _permit = permit;
                let Ok(mut tcp) =
                    tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await
                else {
                    return;
                };
                let mut stream = tokio::io::join(recv, send);
                let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
            });
        }
        Ok(())
    }
}

/// The exec command that carries the frame protocol (shell contract
/// transport `ssh-exec`).
pub const WORLD_EXEC_COMMAND: &str = "id-world/1";
/// Unread frame bytes one exec channel may queue before the host ends its
/// session. The handler must never wait for space (it shares the loop that
/// flushes output), and dropping bytes would corrupt the framing, so a flood
/// ends the session instead.
const MAX_EXEC_BACKLOG: usize = 8 * crate::frames::MAX_FRAME_BYTES;
/// Widest viewport a `resize` frame may name (shell contract v1).
const MAX_VIEWPORT_WIDTH: u32 = 500;
/// Tallest viewport a `resize` frame may name (shell contract v1).
const MAX_VIEWPORT_HEIGHT: u32 = 200;

/// Serve SSH connections until the listener fails.
pub async fn serve(
    listener: TcpListener,
    hub: WorldHub,
    host_key: PrivateKey,
    blobs: iroh_blobs::api::Store,
) -> std::io::Result<()> {
    serve_limited(listener, hub, host_key, blobs, MAX_CONNECTIONS).await
}

async fn serve_limited(
    listener: TcpListener,
    hub: WorldHub,
    host_key: PrivateKey,
    blobs: iroh_blobs::api::Store,
    max_connections: usize,
) -> std::io::Result<()> {
    let config = Arc::new(Config {
        keys: vec![host_key],
        methods: MethodSet::from(&[MethodKind::Password, MethodKind::PublicKey][..]),
        maximum_packet_size: MAX_PACKET_BYTES,
        inactivity_timeout: Some(INACTIVITY_TIMEOUT),
        ..Config::default()
    });
    let mut server = SshServer {
        hub,
        blobs,
        connections: Arc::new(Semaphore::new(max_connections)),
    };
    server.run_on_socket(config, &listener).await
}

/// The fingerprint clients see for the host key, for trust-on-first-use checks.
pub fn host_key_fingerprint(key: &PrivateKey) -> String {
    key.public_key().fingerprint(HashAlg::Sha256).to_string()
}

/// The SSH host key, created on first use and reused on every later start.
pub async fn load_or_create_host_key(path: &str) -> anyhow::Result<PrivateKey> {
    match tokio::fs::read_to_string(path).await {
        Ok(pem) => Ok(PrivateKey::from_openssh(pem)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
            let pem = key.to_openssh(LineEnding::LF)?;
            crate::store::write_private_file(path, pem.as_bytes()).await?;
            Ok(key)
        }
        Err(error) => Err(error.into()),
    }
}

#[derive(Clone)]
struct SshServer {
    hub: WorldHub,
    blobs: iroh_blobs::api::Store,
    connections: Arc<Semaphore>,
}

impl Server for SshServer {
    type Handler = SshClient;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> SshClient {
        SshClient {
            hub: self.hub.clone(),
            blobs: self.blobs.clone(),
            permit: Arc::clone(&self.connections).try_acquire_owned().ok(),
            login: None,
            input: None,
            viewport: None,
            dropped_input: Arc::new(AtomicUsize::new(0)),
            exec: None,
            exec_backlog: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellMode {
    Raw,
    Command,
}

struct Login {
    world: Option<String>,
    capability: String,
    explore: bool,
    command_mode: bool,
    /// The public key the client proved, as hex. Only explorers have one.
    key: Option<String>,
}

struct SshClient {
    hub: WorldHub,
    blobs: iroh_blobs::api::Store,
    permit: Option<OwnedSemaphorePermit>,
    login: Option<Login>,
    input: Option<mpsc::Sender<SshInputFrame>>,
    viewport: Option<(u32, u32)>,
    /// Bytes the input queue refused, shared with the shell so the drop is
    /// visible. The handler counts drops for the life of the connection,
    /// including after the shell opens, so the counter must be shared, not
    /// copied, when the shell is constructed.
    dropped_input: Arc<AtomicUsize>,
    /// Bytes for the frame session on an exec channel, if one runs.
    exec: Option<mpsc::UnboundedSender<Vec<u8>>>,
    /// Bytes queued on `exec` and not yet read by the session.
    exec_backlog: Arc<AtomicUsize>,
}

enum SshInputFrame {
    Bytes(Vec<u8>),
    Resize(u32, u32),
}

impl Handler for SshClient {
    type Error = russh::Error;

    // The join capability is checked when the shell opens, so every password
    // is accepted here and a wrong one is answered by the world itself.
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, russh::Error> {
        if self.permit.is_none() {
            return Ok(Auth::reject());
        }
        let (world, explore, command_mode) = if let Some(world) = user.strip_prefix(EXPLORE_PREFIX)
        {
            (world, true, false)
        } else if let Some(world) = user.strip_prefix(COMMAND_PREFIX) {
            (world, false, true)
        } else {
            (user, false, false)
        };
        self.login = Some(Login {
            world: (!world.is_empty()).then(|| world.to_owned()),
            capability: password.to_owned(),
            explore,
            command_mode,
            key: None,
        });
        Ok(Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, russh::Error> {
        let Some(world) = user.strip_prefix(EXPLORE_PREFIX) else {
            return Ok(Auth::reject());
        };
        let Some(ed25519) = public_key.key_data().ed25519() else {
            return Ok(Auth::reject());
        };
        if self.permit.is_none() {
            return Ok(Auth::reject());
        }
        self.login = Some(Login {
            world: (!world.is_empty()).then(|| world.to_owned()),
            capability: String::new(),
            explore: true,
            command_mode: false,
            key: Some(hex_encode(&ed25519.0)),
        });
        Ok(Auth::Accept)
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), russh::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn channel_open_x11(
        &mut self,
        _channel: Channel<Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), russh::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        // The frame protocol is the only command, and one session (shell or
        // frames) runs per connection.
        if data != WORLD_EXEC_COMMAND.as_bytes()
            || self.login.is_none()
            || self.input.is_some()
            || self.exec.is_some()
        {
            return session.channel_failure(channel);
        }
        let (exec, bytes) = mpsc::unbounded_channel();
        self.exec = Some(exec);
        session.channel_success(channel)?;
        let mut io = SshFrameIo {
            handle: session.handle(),
            channel,
            bytes,
            buffer: FrameBuffer::default(),
            backlog: Arc::clone(&self.exec_backlog),
        };
        let hub = self.hub.clone();
        tokio::spawn(async move {
            run_session(&hub, &mut io).await;
            let _ = io.handle.eof(channel).await;
            let _ = io.handle.close(channel).await;
        });
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _name: &str,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        session.channel_failure(channel)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), russh::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        self.viewport = Some((col_width, row_height));
        session.channel_success(channel)
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        self.viewport = Some((col_width, row_height));
        if let Some(input) = &self.input {
            let _ = input.try_send(SshInputFrame::Resize(col_width, row_height));
        }
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        let Some(login) = self
            .login
            .as_ref()
            .filter(|_| self.input.is_none() && self.exec.is_none())
        else {
            return session.channel_failure(channel);
        };
        if login.explore {
            let explorer = Explorer::new(self.hub.clone(), login, &login.capability);
            let (input, bytes) = mpsc::channel(INPUT_QUEUE);
            self.input = Some(input);
            let handle = session.handle();
            session.channel_success(channel)?;
            let dropped_input = Arc::clone(&self.dropped_input);
            tokio::spawn(run_explorer(
                explorer,
                handle,
                channel,
                bytes,
                dropped_input,
            ));
            return Ok(());
        }
        let first = join_frame(login);
        session.channel_success(channel)?;
        let alt_screen = !login.command_mode;

        let (input, frames) = mpsc::channel(INPUT_QUEUE);
        let first_resize = self.viewport.take();
        self.input = Some(input);
        let mode = if login.command_mode {
            ShellMode::Command
        } else {
            ShellMode::Raw
        };
        let mut io = SshIo {
            handle: session.handle(),
            channel,
            frames,
            blobs: self.blobs.clone(),
            first: Some(first),
            first_resize,
            capability: login.capability.clone(),
            mode,
            alt_screen,
            alt_screen_entered: false,
            last_error: None,
            dropped_input: Arc::clone(&self.dropped_input),
            lines: WorldLineBuffer::default(),
            commands: VecDeque::new(),
            records_prefix: None,
            records_drained: 0,
            errored: false,
            previous_screen: None,
        };
        let hub = self.hub.clone();
        tokio::spawn(async move {
            io.enter_alt_screen().await;
            run_session(&hub, &mut io).await;
            if io.errored {
                tokio::time::sleep(REFUSED_SHELL_DELAY).await;
            }
            io.leave_alt_screen().await;
            let _ = io.handle.close(channel).await;
        });
        Ok(())
    }

    async fn data(
        &mut self,
        _channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), russh::Error> {
        if let Some(exec) = &self.exec {
            let queued = self.exec_backlog.fetch_add(data.len(), Ordering::Relaxed) + data.len();
            if queued > MAX_EXEC_BACKLOG || exec.send(data.to_vec()).is_err() {
                tracing::warn!(
                    "world ssh exec backlog over {MAX_EXEC_BACKLOG} bytes; ending session"
                );
                self.exec = None;
            }
            return Ok(());
        }
        let Some(input) = &self.input else {
            return Ok(());
        };
        // Never wait for queue space: this handler runs on the same loop that
        // flushes the world's output, so waiting here could deadlock.
        if let Err(error) = input.try_send(SshInputFrame::Bytes(data.to_vec())) {
            match error {
                mpsc::error::TrySendError::Full(_) => {
                    // The handler shares the loop that flushes output, so it must
                    // not wait for queue space. Count the loss on the counter
                    // shared with the shell instead, and let the shell tell the
                    // viewer once, instead of dropping silently.
                    let total = record_dropped_input(&self.dropped_input, data.len());
                    tracing::warn!(
                        "world ssh input queue full; dropped {} bytes ({} total)",
                        data.len(),
                        total
                    );
                }
                mpsc::error::TrySendError::Closed(_) => self.input = None,
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), russh::Error> {
        self.input = None;
        self.exec = None;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), russh::Error> {
        self.input = None;
        self.exec = None;
        Ok(())
    }
}

/// The session frame one raw terminal event becomes: keystrokes are
/// `input`, a window change is `resize` (shell contract, joined phase).
fn raw_frame(input: &SshInputFrame) -> String {
    match input {
        SshInputFrame::Bytes(bytes) => {
            json!({ "type": "input", "data_hex": encode_hex(bytes) }).to_string()
        }
        // A terminal outside the contract's viewport bounds still gets a
        // view: it is clamped instead of refused.
        SshInputFrame::Resize(width, height) => json!({
            "type": "resize",
            "width": (*width).clamp(1, MAX_VIEWPORT_WIDTH),
            "height": (*height).clamp(1, MAX_VIEWPORT_HEIGHT),
        })
        .to_string(),
    }
}

/// The session frame a command-mode line sends, or `None` for the commands
/// the shell answers itself (`/help`, `/logs`, `/documents`, ...).
fn command_frame(command: &WorldCommand, capability: &str) -> Option<String> {
    let frame = match command {
        WorldCommand::Chat(text) => json!({ "type": "chat", "text": text }),
        WorldCommand::Input(data_hex) => json!({ "type": "input", "data_hex": data_hex }),
        WorldCommand::Records { prefix } => json!({
            "type": "records", "capability": capability, "prefix": prefix, "after": null
        }),
        WorldCommand::RecordsContinue { prefix, after } => json!({
            "type": "records", "capability": capability, "prefix": prefix, "after": after
        }),
        WorldCommand::Stats => json!({ "type": "info", "capability": capability }),
        WorldCommand::Caps => json!({ "type": "caps", "capability": capability }),
        WorldCommand::Documents { .. }
        | WorldCommand::Document(_)
        | WorldCommand::Logs { .. }
        | WorldCommand::Help
        | WorldCommand::Message(_)
        | WorldCommand::Quit => return None,
    };
    Some(frame.to_string())
}

fn join_frame(login: &Login) -> String {
    let mut join = json!({ "type": "join", "capability": login.capability });
    if let Some(world) = &login.world {
        join["world"] = json!(world);
    }
    join.to_string()
}

/// [`SessionIo`] for the frame protocol on an exec channel: the same
/// envelope as the iroh stream, reassembled from the channel's data packets.
struct SshFrameIo {
    handle: Handle,
    channel: ChannelId,
    bytes: mpsc::UnboundedReceiver<Vec<u8>>,
    buffer: FrameBuffer,
    backlog: Arc<AtomicUsize>,
}

impl SessionIo for SshFrameIo {
    // Cancel-safe: state changes only after a chunk has been received.
    async fn recv(&mut self) -> Inbound {
        loop {
            match self.buffer.next_frame() {
                Some(Decoded::Frame(body)) => {
                    return String::from_utf8(body).map_or(Inbound::Unsupported, Inbound::Text);
                }
                // As on iroh, an oversized length ends the stream unread.
                Some(Decoded::Oversized) => return Inbound::Closed,
                None => {}
            }
            let Some(chunk) = self.bytes.recv().await else {
                return Inbound::Closed;
            };
            self.backlog.fetch_sub(chunk.len(), Ordering::Relaxed);
            self.buffer.push(&chunk);
        }
    }

    async fn send(&mut self, frame: String) -> bool {
        match encode_frame(frame.as_bytes()) {
            Ok(bytes) => self.handle.data(self.channel, bytes).await.is_ok(),
            Err(_) => false,
        }
    }
}

/// [`SessionIo`] for one SSH shell: the join frame first, then terminal lines
/// as chat/command frames, and terminal output for each view or record page.
struct SshIo {
    handle: Handle,
    channel: ChannelId,
    frames: mpsc::Receiver<SshInputFrame>,
    blobs: iroh_blobs::api::Store,
    first: Option<String>,
    first_resize: Option<(u32, u32)>,
    capability: String,
    mode: ShellMode,
    /// Whether this session uses the alternate screen buffer (raw sessions only;
    /// command mode never emits either sequence).
    alt_screen: bool,
    /// Whether the enter sequence has actually been written to the channel.
    alt_screen_entered: bool,
    /// The last error message, re-sent after leaving the alternate screen.
    last_error: Option<String>,
    /// Bytes the input queue refused, shared with the russh handler: drops
    /// counted after the shell opens are exactly the ones this session must
    /// report, so the shell reads the live counter, not a snapshot.
    dropped_input: Arc<AtomicUsize>,
    lines: WorldLineBuffer,
    commands: VecDeque<WorldCommand>,
    /// `Some(prefix)` while draining records pages from a `/records` command.
    records_prefix: Option<Option<String>>,
    /// Cumulative bytes already rendered for the current `/records` drain, so a
    /// large world's records are bounded across pages, not just per page.
    records_drained: usize,
    errored: bool,
    /// Last screen successfully written to this shell's terminal. A missing
    /// value forces the next screen view to repaint the whole terminal.
    previous_screen: Option<crate::world_presentation::ScreenSurface>,
}

/// The notice shown to a viewer once [`INPUT_DROP_NOTICE_BYTES`] input bytes
/// were refused. Empty below that threshold, so a drop here and there stays
/// quiet and only a sustained loss is reported.
fn dropped_input_notice(bytes: usize) -> Option<String> {
    (bytes >= INPUT_DROP_NOTICE_BYTES).then(|| {
        format!("\r\n[-- input dropped: {bytes} bytes refused by the SSH input queue --]\r\n")
    })
}

/// Count bytes the input queue refused on the counter shared between the
/// russh handler and the session I/O; returns the new total.
fn record_dropped_input(counter: &AtomicUsize, bytes: usize) -> usize {
    counter.fetch_add(bytes, Ordering::Relaxed) + bytes
}

/// Claim the pending dropped-input notice, if the shared counter has reached
/// [`INPUT_DROP_NOTICE_BYTES`]. Claiming subtracts only the observed bytes,
/// so drops counted while the notice is being handed to the viewer stay
/// counted for the next one. If the notice cannot be delivered, the caller
/// adds the claimed bytes back with `fetch_add`, so claimed drops are reset
/// only once the notice has actually reached the viewer: they are neither
/// double-counted nor lost between frames.
fn claim_dropped_input_notice(counter: &AtomicUsize) -> Option<(usize, String)> {
    let bytes = counter.load(Ordering::Relaxed);
    let notice = dropped_input_notice(bytes)?;
    counter.fetch_sub(bytes, Ordering::Relaxed);
    Some((bytes, notice))
}

/// Prepend the pending dropped-input notice to the explorer's next output,
/// claiming it from the shared counter. Returns the claimed byte count so
/// the caller can restore it with `fetch_add` if the output write fails —
/// the same reset-only-on-delivery rule as [`SshIo::send`]. Below the
/// threshold the output is left untouched and the drops stay counted.
fn prepend_dropped_input_notice(counter: &AtomicUsize, out: &mut Vec<u8>) -> Option<usize> {
    let (bytes, notice) = claim_dropped_input_notice(counter)?;
    let mut combined = notice.into_bytes();
    combined.append(out);
    *out = combined;
    Some(bytes)
}

impl SshIo {
    /// Enter the alternate screen before the first session frame, so a raw
    /// session's repaints never land in the viewer's scrollback.
    async fn enter_alt_screen(&mut self) {
        if !self.alt_screen || self.alt_screen_entered {
            return;
        }
        if self
            .handle
            .data(self.channel, ALT_SCREEN_ENTER)
            .await
            .is_ok()
        {
            self.alt_screen_entered = true;
        }
    }

    /// Leave the alternate screen and re-send the last error after the leave
    /// sequence, so a refused join stays readable in the restored scrollback.
    async fn leave_alt_screen(&mut self) {
        if !self.alt_screen_entered {
            return;
        }
        self.alt_screen_entered = false;
        let text = match self.last_error.take() {
            Some(error) => format!("{ALT_SCREEN_LEAVE}\r\n{error}\r\n"),
            None => ALT_SCREEN_LEAVE.to_owned(),
        };
        let _ = self.handle.data(self.channel, text).await;
    }
}

impl SessionIo for SshIo {
    async fn recv(&mut self) -> Inbound {
        if let Some(join) = self.first.take() {
            return Inbound::Text(join);
        }
        if let Some((width, height)) = self.first_resize.take() {
            self.previous_screen = None;
            return Inbound::Text(raw_frame(&SshInputFrame::Resize(width, height)));
        }
        match self.mode {
            ShellMode::Raw => self
                .frames
                .recv()
                .await
                .map_or(Inbound::Closed, |input| Inbound::Text(raw_frame(&input))),
            ShellMode::Command => loop {
                if let Some(command) = self.commands.pop_front() {
                    if let Some(frame) = command_frame(&command, &self.capability) {
                        if let WorldCommand::Records { prefix } = &command {
                            self.records_prefix = Some(prefix.clone());
                            self.records_drained = 0;
                        }
                        return Inbound::Text(frame);
                    }
                    // Everything else is answered by the shell itself.
                    match command {
                        WorldCommand::Documents { prefix } => {
                            let text =
                                match list_public_documents(&self.blobs, prefix.as_deref()).await {
                                    Ok(text) => text,
                                    Err(_) => "documents unavailable".to_owned(),
                                };
                            if self
                                .handle
                                .data(
                                    self.channel,
                                    terminal_text(&format!("\r\n{text}\r\nworld> ")),
                                )
                                .await
                                .is_err()
                            {
                                return Inbound::Closed;
                            }
                        }
                        WorldCommand::Document(name) => {
                            let text = match read_public_document(&self.blobs, &name).await {
                                Ok(text) => format!("\r\n{text}\r\nworld> "),
                                Err(error) => format!("\r\n{}\r\nworld> ", error.message()),
                            };
                            if self
                                .handle
                                .data(self.channel, terminal_text(&text))
                                .await
                                .is_err()
                            {
                                return Inbound::Closed;
                            }
                        }
                        WorldCommand::Help => {
                            let help = "World chat: type a line to chat. Commands: /input <hex>, /records [prefix], /stats, /logs, /caps, /documents [prefix], /document <name>, /help, /quit\r\nworld> ";
                            if self.handle.data(self.channel, help).await.is_err() {
                                return Inbound::Closed;
                            }
                        }
                        WorldCommand::Logs { seconds } => {
                            let window = match seconds {
                                Some(seconds) => {
                                    format!(" (the requested {seconds}s window is ignored here)")
                                }
                                None => String::new(),
                            };
                            let text = format!(
                                "event tailing is not available in command mode{window}; join in raw mode (no `cmd:` prefix) to see events live, or use `id world logs`\r\nworld> "
                            );
                            let text = text.as_str();
                            if self
                                .handle
                                .data(self.channel, terminal_text(text))
                                .await
                                .is_err()
                            {
                                return Inbound::Closed;
                            }
                        }
                        WorldCommand::Message(message) => {
                            let text = format!("{message}\r\nworld> ");
                            if self.handle.data(self.channel, text).await.is_err() {
                                return Inbound::Closed;
                            }
                        }
                        WorldCommand::Quit => return Inbound::Closed,
                        WorldCommand::Chat(_)
                        | WorldCommand::Input(_)
                        | WorldCommand::Records { .. }
                        | WorldCommand::RecordsContinue { .. }
                        | WorldCommand::Stats
                        | WorldCommand::Caps => {}
                    }
                }
                match self.frames.recv().await {
                    Some(SshInputFrame::Bytes(bytes)) => {
                        self.commands.extend(self.lines.push(&bytes));
                    }
                    Some(resize @ SshInputFrame::Resize(..)) => {
                        self.previous_screen = None;
                        return Inbound::Text(raw_frame(&resize));
                    }
                    None => return Inbound::Closed,
                }
            },
        }
    }

    async fn send(&mut self, frame: String) -> bool {
        // Tell the viewer about dropped input before the frame it delayed.
        // The count is claimed from the shared counter first; if the notice
        // itself cannot be written, the claim is restored so the drops stay
        // counted instead of vanishing with the undelivered notice.
        if let Some((bytes, notice)) = claim_dropped_input_notice(&self.dropped_input) {
            if self.handle.data(self.channel, notice).await.is_err() {
                self.dropped_input.fetch_add(bytes, Ordering::Relaxed);
                return false;
            }
            self.previous_screen = None;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&frame) else {
            return true;
        };
        let mut next_screen = None;
        let text = match value["type"].as_str() {
            Some("snapshot") => {
                if matches!(self.mode, ShellMode::Command) {
                    "World chat: type a line to chat; /help lists commands.\r\nworld> ".to_owned()
                } else {
                    return true;
                }
            }
            Some("view") => {
                let rendered = if let Some(surface) = value.get("surface") {
                    if let Ok(crate::world_presentation::WorldSurface::Screen(screen)) =
                        serde_json::from_value(surface.clone())
                    {
                        let raw_mode = matches!(self.mode, ShellMode::Raw);
                        let rendered = if raw_mode {
                            self.previous_screen
                                .as_ref()
                                .and_then(|previous| screen.render_ansi_diff(previous))
                                .or_else(|| screen.render_ansi())
                        } else {
                            screen.render_ansi()
                        };
                        if let Some(rendered) = rendered {
                            if raw_mode {
                                next_screen = Some(screen);
                            }
                            rendered
                        } else if let Some(data) = value["data"].as_str() {
                            format!("\x1b[H\x1b[2J{}", terminal_text(data))
                        } else {
                            return true;
                        }
                    } else if let Some(data) = value["data"].as_str() {
                        format!("\x1b[H\x1b[2J{}", terminal_text(data))
                    } else {
                        return true;
                    }
                } else if let Some(data) = value["data"].as_str() {
                    format!("\x1b[H\x1b[2J{}", terminal_text(data))
                } else {
                    return true;
                };

                if matches!(self.mode, ShellMode::Command) {
                    format!(
                        "\x1b[H\x1b[2J{}\r\nType a line to chat; /help lists commands.\r\nworld> ",
                        rendered
                    )
                } else {
                    rendered
                }
            }
            Some("error") => {
                self.errored = true;
                self.records_prefix = None;
                let message = value["message"].as_str().unwrap_or("error");
                self.last_error = Some(format!("\r\n{}\r\n", terminal_text(message)));
                self.last_error.clone().unwrap_or_default()
            }
            Some("records") => {
                let records = value["records"].as_object();
                let sequence = value["sequence"].as_u64().unwrap_or_default();
                let after = value["next_after"].as_str().map(str::to_owned);
                let text = render_records_value(records, sequence, after.as_deref());
                // Bound the whole drain, not just each page: a world with many
                // records would otherwise keep fetching pages without limit.
                self.records_drained = self.records_drained.saturating_add(text.len());
                let next = match (&self.records_prefix, &after) {
                    (Some(prefix), Some(after))
                        if self.records_drained < MAX_RECORDS_TOTAL_BYTES =>
                    {
                        Some((prefix.clone(), after.clone()))
                    }
                    _ => None,
                };
                let truncated = after.is_some() && self.records_drained >= MAX_RECORDS_TOTAL_BYTES;
                if let Some((prefix, after)) = next {
                    self.commands
                        .push_back(WorldCommand::RecordsContinue { prefix, after });
                } else {
                    self.records_prefix = None;
                }
                if truncated {
                    format!(
                        "\r\n{}\r\n{}",
                        terminal_text(&text),
                        terminal_text("[records truncated: too much output]\n")
                    )
                } else {
                    format!("\r\n{}", terminal_text(&text))
                }
            }
            Some("caps") => {
                let report = value
                    .get("report")
                    .map(serde_json::to_string_pretty)
                    .transpose()
                    .unwrap_or_default()
                    .unwrap_or_else(|| "caps report unavailable".to_owned());
                format!("\r\n{report}\r\nworld> ")
            }
            Some("module_info") => {
                let info = serde_json::to_string_pretty(&value).unwrap_or_default();
                format!("\r\n{info}\r\nworld> ")
            }
            _ => return true,
        };
        if text.is_empty() {
            return true;
        }
        let written = self.handle.data(self.channel, text).await.is_ok();
        if written {
            // Non-view output invalidates the terminal baseline; a successfully
            // written screen view becomes the new baseline only now.
            self.previous_screen = next_screen;
        }
        written
    }
}

/// Where `id world join --via ssh` connects: `[user@]host[:port]` (port 22
/// by default; an IPv6 address goes in brackets). The user is only the SSH
/// login: on the frame channel it names no world and grants nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshTarget {
    /// SSH login name (`id` when absent).
    pub user: String,
    /// Host name or address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

impl std::str::FromStr for SshTarget {
    type Err = anyhow::Error;

    fn from_str(text: &str) -> anyhow::Result<Self> {
        let (user, rest) = text.rsplit_once('@').unwrap_or(("id", text));
        let port = |digits: &str| {
            digits
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("`{digits}` is not a port"))
        };
        let (host, port) = if let Some(inner) = rest.strip_prefix('[') {
            let Some((host, tail)) = inner.split_once(']') else {
                anyhow::bail!("`{text}`: missing `]` after the IPv6 address");
            };
            match tail.strip_prefix(':') {
                Some(digits) => (host, port(digits)?),
                None if tail.is_empty() => (host, 22),
                None => anyhow::bail!("`{text}`: expected `:port` after `]`"),
            }
        } else {
            match rest.split_once(':') {
                Some((host, digits)) if !digits.contains(':') => (host, port(digits)?),
                Some(_) => anyhow::bail!("`{text}`: put an IPv6 address in brackets"),
                None => (rest, 22),
            }
        };
        anyhow::ensure!(
            !host.is_empty() && !user.is_empty(),
            "`{text}` is not [user@]host[:port]"
        );
        Ok(Self {
            user: user.to_owned(),
            host: host.to_owned(),
            port,
        })
    }
}

/// How the SSH client decides to trust the host key. There is no
/// trust-on-first-use: an unknown key is refused with its fingerprint.
#[derive(Clone, Debug)]
pub enum HostKeyCheck {
    /// The SHA256 fingerprint `id serve` prints (`SHA256:...`).
    Fingerprint(String),
    /// An OpenSSH `known_hosts` file.
    KnownHosts(std::path::PathBuf),
}

/// The client half of the host-key decision.
#[derive(Debug)]
pub struct HostKeyTrust {
    host: String,
    port: u16,
    check: HostKeyCheck,
    refused: Arc<std::sync::Mutex<Option<String>>>,
}

impl russh::client::Handler for HostKeyTrust {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, russh::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            self.refuse("a certificate (not supported)".to_owned());
            return Ok(false);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let trusted = match &self.check {
            HostKeyCheck::Fingerprint(expected) => *expected == fingerprint,
            HostKeyCheck::KnownHosts(path) => {
                russh::keys::check_known_hosts_path(&self.host, self.port, key, path)
                    .unwrap_or(false)
            }
        };
        if !trusted {
            self.refuse(fingerprint);
        }
        Ok(trusted)
    }
}

impl HostKeyTrust {
    fn refuse(&self, fingerprint: String) {
        if let Ok(mut refused) = self.refused.lock() {
            *refused = Some(fingerprint);
        }
    }
}

/// Keeps the SSH connection of a frame session open; dropping it disconnects.
pub struct SshConnection {
    _session: russh::client::Handle<HostKeyTrust>,
}

impl std::fmt::Debug for SshConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshConnection").finish_non_exhaustive()
    }
}

/// Open a frame session over an SSH exec channel. The returned stream
/// carries exactly the bytes an iroh `/id-world/1` stream would: use
/// [`crate::frames::write_frame_to`] and [`crate::frames::read_frame_from`].
///
/// # Errors
///
/// Fails if the host is unreachable, its key is not trusted, or it does not
/// offer the frame transport.
pub async fn connect_frames(
    target: &SshTarget,
    check: HostKeyCheck,
) -> anyhow::Result<(SshConnection, russh::ChannelStream<russh::client::Msg>)> {
    use anyhow::Context as _;

    let refused = Arc::new(std::sync::Mutex::new(None));
    let trust = HostKeyTrust {
        host: target.host.clone(),
        port: target.port,
        check,
        refused: Arc::clone(&refused),
    };
    let config = Arc::new(russh::client::Config::default());
    let mut session =
        match russh::client::connect(config, (target.host.as_str(), target.port), trust).await {
            Ok(session) => session,
            Err(error) => {
                if let Some(key) = refused.lock().ok().and_then(|mut refused| refused.take()) {
                    anyhow::bail!(
                        "the host key of {}:{} ({key}) is not trusted; compare it with the \
                         `world ssh host key` line `id serve` prints, then pass \
                         --ssh-host-key {key} or add the key to known_hosts",
                        target.host,
                        target.port
                    );
                }
                return Err(error).context("connect over ssh");
            }
        };
    // The login grants nothing here: the frames carry their own capability.
    let auth = session
        .authenticate_password(target.user.as_str(), "")
        .await
        .context("ssh login")?;
    anyhow::ensure!(auth.success(), "the ssh server refused the login");
    let mut channel = session
        .channel_open_session()
        .await
        .context("open ssh channel")?;
    channel
        .exec(true, WORLD_EXEC_COMMAND)
        .await
        .context("start the frame transport")?;
    loop {
        match channel.wait().await {
            Some(russh::ChannelMsg::Success) => break,
            Some(russh::ChannelMsg::Failure) | None => anyhow::bail!(
                "the ssh server does not offer the id world frame transport (exec {WORLD_EXEC_COMMAND})"
            ),
            Some(_) => {}
        }
    }
    Ok((SshConnection { _session: session }, channel.into_stream()))
}

const MAX_WORLD_TERMINAL_LINE: usize = 1024;
const MAX_RECORDS_TERMINAL_BYTES: usize = 384 * 1024;
/// Total rendered bytes a single `/records` drain may produce across all its
/// pages. Each page is additionally bounded by [`MAX_RECORDS_TERMINAL_BYTES`].
const MAX_RECORDS_TOTAL_BYTES: usize = 1024 * 1024;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const MAX_DOCUMENT_LIST_BYTES: usize = 32 * 1024;
const MAX_DOCUMENT_LIST_ITEMS: usize = 512;

#[derive(Debug, PartialEq, Eq)]
enum WorldCommand {
    Chat(String),
    Input(String),
    Records {
        prefix: Option<String>,
    },
    RecordsContinue {
        prefix: Option<String>,
        after: String,
    },
    Documents {
        prefix: Option<String>,
    },
    Document(String),
    Stats,
    Logs {
        seconds: Option<u64>,
    },
    Caps,
    Help,
    Message(String),
    Quit,
}

fn world_command(line: &str) -> Option<WorldCommand> {
    use crate::world_line::{WorldLine, parse_line};
    Some(match parse_line(line)? {
        WorldLine::Chat(text) => WorldCommand::Chat(text),
        WorldLine::Input(hex) => WorldCommand::Input(hex),
        WorldLine::Records { prefix } => WorldCommand::Records { prefix },
        WorldLine::Documents { prefix } => WorldCommand::Documents { prefix },
        WorldLine::Document(name) => WorldCommand::Document(name),
        WorldLine::Stats => WorldCommand::Stats,
        WorldLine::Logs { seconds } => WorldCommand::Logs { seconds },
        WorldLine::Caps => WorldCommand::Caps,
        WorldLine::Help => WorldCommand::Help,
        WorldLine::Quit => WorldCommand::Quit,
        WorldLine::Message(message) => WorldCommand::Message(message),
    })
}

#[derive(Default)]
struct WorldLineBuffer {
    line: Vec<u8>,
    overflow: bool,
    after_cr: bool,
    escape: WorldEscape,
}

#[derive(Default)]
enum WorldEscape {
    #[default]
    None,
    Start,
    Sequence,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

impl WorldLineBuffer {
    fn push(&mut self, packet: &[u8]) -> Vec<WorldCommand> {
        let mut commands = Vec::new();
        for &byte in packet {
            match self.escape {
                WorldEscape::Start => {
                    self.escape = match byte {
                        b'[' | b'O' => WorldEscape::Sequence,
                        b']' => WorldEscape::Osc,
                        b'P' | b'_' | b'^' => WorldEscape::String,
                        _ => WorldEscape::None,
                    };
                    continue;
                }
                WorldEscape::Sequence => {
                    if (0x40..=0x7e).contains(&byte) {
                        self.escape = WorldEscape::None;
                    }
                    continue;
                }
                WorldEscape::Osc => {
                    if byte == 0x07 {
                        self.escape = WorldEscape::None;
                    } else if byte == 0x1b {
                        self.escape = WorldEscape::OscEscape;
                    }
                    continue;
                }
                WorldEscape::OscEscape => {
                    self.escape = if byte == b'\\' {
                        WorldEscape::None
                    } else {
                        WorldEscape::Osc
                    };
                    continue;
                }
                WorldEscape::String => {
                    if byte == 0x1b {
                        self.escape = WorldEscape::StringEscape;
                    }
                    continue;
                }
                WorldEscape::StringEscape => {
                    self.escape = if byte == b'\\' {
                        WorldEscape::None
                    } else {
                        WorldEscape::String
                    };
                    continue;
                }
                WorldEscape::None => {}
            }
            if byte == 0x1b {
                self.escape = WorldEscape::Start;
                continue;
            }
            let after_cr = std::mem::take(&mut self.after_cr);
            if byte == b'\n' && after_cr {
                continue;
            }
            if byte == b'\r' || byte == b'\n' {
                self.after_cr = byte == b'\r';
                if !self.overflow {
                    let bytes = std::mem::take(&mut self.line);
                    let line = String::from_utf8_lossy(&bytes);
                    if let Some(command) = world_command(&line) {
                        commands.push(command);
                    }
                } else {
                    self.line.clear();
                }
                self.overflow = false;
                continue;
            }
            if byte == 0x03 || (byte == 0x04 && self.line.is_empty()) {
                commands.push(WorldCommand::Quit);
            } else if matches!(byte, 0x7f | 0x08) {
                self.line.pop();
            } else if (0x20..=0x7e).contains(&byte) || byte >= 0x80 {
                if self.line.len() < MAX_WORLD_TERMINAL_LINE && !self.overflow {
                    self.line.push(byte);
                } else {
                    self.overflow = true;
                    self.line.clear();
                }
            }
        }
        commands
    }
}

fn render_records_value(
    records: Option<&serde_json::Map<String, serde_json::Value>>,
    sequence: u64,
    next_after: Option<&str>,
) -> String {
    let mut output = format!("records at sequence {sequence}:\n");
    if let Some(records) = records {
        for (key, value) in records {
            let key = serde_json::to_string(key).unwrap_or_else(|_| "\"?\"".to_owned());
            let value = serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned());
            let line = format!("{key} = {value}\n");
            if output.len() + line.len() + 32 > MAX_RECORDS_TERMINAL_BYTES {
                output.push_str("[page display truncated]\n");
                break;
            }
            output.push_str(&line);
        }
    }
    if next_after.is_some() {
        output.push_str("more records; fetching next page\n");
    }
    output
}

#[cfg(test)]
fn render_records(
    records: &crate::world::Records,
    sequence: u64,
    next_after: Option<&str>,
) -> String {
    let value = serde_json::to_value(records).ok();
    render_records_value(
        value.as_ref().and_then(serde_json::Value::as_object),
        sequence,
        next_after,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentError {
    InvalidName,
    Missing,
    TooLarge,
    Binary,
    Unavailable,
}

impl DocumentError {
    const fn message(self) -> &'static str {
        match self {
            Self::InvalidName => "document name must be a public .txt, .md, or .markdown tag",
            Self::Missing => "document not found",
            Self::TooLarge => "document exceeds the 64 KiB SSH viewing limit",
            Self::Binary => "document is not valid UTF-8 text",
            Self::Unavailable => "document unavailable",
        }
    }
}

fn is_viewable_document_name(name: &str) -> bool {
    if !is_public_document_name(name) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".txt")
}

async fn list_public_documents(
    store: &iroh_blobs::api::Store,
    prefix: Option<&str>,
) -> anyhow::Result<String> {
    let mut tags = store.tags().list().await?;
    let mut names = Vec::new();
    let mut total_bytes = 0usize;
    let mut truncated = false;
    while let Some(item) = tags.next().await {
        let item = item?;
        let Ok(name) = std::str::from_utf8(item.name.as_ref()) else {
            continue;
        };
        if !is_public_document_name(name) || prefix.is_some_and(|p| !name.starts_with(p)) {
            continue;
        }
        let line = format!("{name}\n");
        if names.len() >= MAX_DOCUMENT_LIST_ITEMS
            || total_bytes + line.len() > MAX_DOCUMENT_LIST_BYTES
        {
            truncated = true;
            break;
        }
        total_bytes += line.len();
        names.push(name.to_owned());
    }
    names.sort();
    let mut output = String::from("public named files:\n");
    if names.is_empty() {
        output.push_str("(none)\n");
    } else {
        for name in names {
            output.push_str(&name);
            output.push('\n');
        }
    }
    if truncated {
        output.push_str("[listing truncated]\n");
    }
    output.push_str("View UTF-8 .txt/.md/.markdown files with /document <name>.\n");
    Ok(output)
}

fn is_public_document_name(name: &str) -> bool {
    !name.is_empty()
        && !crate::tags::is_internal_tag(name)
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
}

async fn read_public_document(
    store: &iroh_blobs::api::Store,
    name: &str,
) -> Result<String, DocumentError> {
    if !is_viewable_document_name(name) {
        return Err(DocumentError::InvalidName);
    }
    let tag = store
        .tags()
        .get(name)
        .await
        .map_err(|_| DocumentError::Unavailable)?
        .ok_or(DocumentError::Missing)?;
    let size = match store.blobs().status(tag.hash).await {
        Ok(iroh_blobs::api::blobs::BlobStatus::Complete { size }) => size,
        _ => return Err(DocumentError::Unavailable),
    };
    if size > MAX_DOCUMENT_BYTES as u64 {
        return Err(DocumentError::TooLarge);
    }
    let bytes = store
        .blobs()
        .get_bytes(tag.hash)
        .await
        .map_err(|_| DocumentError::Unavailable)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| DocumentError::Binary)?;
    Ok(format!("--- {name} ({size} bytes; read-only) ---\n{text}"))
}

/// One explorer login: the world, the caller it acts as, and the secret it
/// leases the world with. Sign-in changes the caller for the rest of the login.
struct Explorer {
    hub: WorldHub,
    world: Option<String>,
    token: Option<String>,
    caller: Caller,
}

impl Explorer {
    fn new(hub: WorldHub, login: &Login, secret: &str) -> Self {
        let mut caller = if hub.admin_ok(secret) {
            Caller {
                admin: true,
                ..Caller::default()
            }
        } else {
            Caller::from_secret(secret)
        };
        caller.key.clone_from(&login.key);
        Self {
            hub,
            world: login.world.clone(),
            token: (!secret.is_empty()).then(|| secret.to_owned()),
            caller,
        }
    }

    async fn act(&self, action: DirectoryAction) -> anyhow::Result<DirectoryOutcome> {
        let lease = self
            .hub
            .lease(self.world.as_deref(), self.token.as_deref())
            .await
            .map_err(|error| match error {
                ResolveError::Busy | ResolveError::TooManyWorlds => {
                    anyhow::anyhow!("the server is busy; try again")
                }
                _ => anyhow::anyhow!("no such world"),
            })?;
        lease.service().directory(self.caller.clone(), action).await
    }

    /// Run one typed line. Returns the text to show, and whether to close.
    async fn execute(&mut self, line: &str) -> (String, bool) {
        let action = match parse_line(line) {
            Ok(Line::Blank) => return (String::new(), false),
            Ok(Line::Help) => return (terminal_text(&format!("{HELP}\n")), false),
            Ok(Line::Quit) => return (terminal_text("bye\n"), true),
            Ok(Line::Action(action)) => action,
            Err(error) => return (terminal_text(&format!("error: {error:#}\n")), false),
        };
        let signs_out = matches!(action, DirectoryAction::SignOut);
        let mailed_to = match &action {
            DirectoryAction::AddEmail { address } | DirectoryAction::SignInEmail { address } => {
                Some(address.clone())
            }
            _ => None,
        };
        let outcome = match self.act(action).await {
            Ok(outcome) => outcome,
            Err(error) => return (terminal_text(&format!("error: {error:#}\n")), false),
        };
        let mut text = String::new();
        if let Some(credential) = outcome.credential {
            let _ = writeln!(text, "credential (shown once): {credential}");
            self.become_caller(Caller {
                credential: Some(credential),
                ..Caller::default()
            });
        }
        if let Some(token) = outcome.session {
            let _ = writeln!(text, "session (shown once): {token}");
            self.become_caller(Caller {
                session: Some(token),
                ..Caller::default()
            });
        }
        if signs_out {
            self.become_caller(Caller::default());
        }
        if let (true, Some(address)) = (outcome.mailed, mailed_to) {
            let _ = writeln!(text, "a code was mailed to {address}");
        }
        text.push_str(&render_text(&outcome.view));
        (terminal_text(&text), false)
    }

    /// Switch to acting as another caller, keeping only this login's key. The
    /// admin token does not carry over: a new identity is not the admin.
    fn become_caller(&mut self, caller: Caller) {
        self.caller = Caller {
            key: self.caller.key.clone(),
            ..caller
        };
    }
}

async fn run_explorer(
    mut explorer: Explorer,
    handle: Handle,
    channel: ChannelId,
    mut bytes: mpsc::Receiver<SshInputFrame>,
    dropped_input: Arc<AtomicUsize>,
) {
    let greeting = match explorer.act(DirectoryAction::View).await {
        Ok(outcome) => format!(
            "{}type help for commands\n{PROMPT}",
            render_text(&outcome.view)
        ),
        Err(error) => {
            tokio::time::sleep(REFUSED_SHELL_DELAY).await;
            let _ = handle
                .data(channel, terminal_text(&format!("{error:#}\n")))
                .await;
            let _ = handle.close(channel).await;
            return;
        }
    };
    if handle
        .data(channel, terminal_text(&greeting))
        .await
        .is_err()
    {
        return;
    }
    let mut typing = Typing::default();
    'input: while let Some(frame) = bytes.recv().await {
        let SshInputFrame::Bytes(chunk) = frame else {
            continue;
        };
        let mut out = Vec::new();
        let mut done = false;
        for byte in chunk {
            match typing.feed(byte) {
                Typed::Nothing => {}
                Typed::Echo(echo) => out.extend(echo),
                Typed::Submit(line) => {
                    out.extend_from_slice(b"\r\n");
                    let (text, quit) = explorer.execute(&line).await;
                    out.extend_from_slice(text.as_bytes());
                    if quit {
                        done = true;
                        break;
                    }
                    out.extend_from_slice(PROMPT.as_bytes());
                }
                Typed::Quit => {
                    out.extend_from_slice(b"\r\nbye\r\n");
                    done = true;
                    break;
                }
            }
        }
        // Tell the explorer viewer about dropped input before the output
        // it delayed, claiming the notice from the counter shared with the
        // handler. If the write fails, restore the claim so the drops stay
        // counted instead of vanishing with the undelivered notice.
        let claimed = prepend_dropped_input_notice(&dropped_input, &mut out);
        if !out.is_empty()
            && handle
                .data(channel, String::from_utf8_lossy(&out).into_owned())
                .await
                .is_err()
        {
            if let Some(bytes) = claimed {
                dropped_input.fetch_add(bytes, Ordering::Relaxed);
            }
            break 'input;
        }
        if done {
            break;
        }
    }
    let _ = handle.close(channel).await;
}

#[derive(Default)]
enum Escape {
    #[default]
    None,
    Start,
    Sequence,
}

enum Typed {
    Nothing,
    Echo(Vec<u8>),
    Submit(String),
    Quit,
}

/// Line editing for the explorer's terminal: printable keys are echoed and
/// kept, backspace removes one, and escape sequences such as arrow keys are
/// dropped.
#[derive(Default)]
struct Typing {
    line: Vec<u8>,
    escape: Escape,
    after_cr: bool,
}

impl Typing {
    fn feed(&mut self, byte: u8) -> Typed {
        match self.escape {
            Escape::Start => {
                self.escape = if matches!(byte, b'[' | b'O') {
                    Escape::Sequence
                } else {
                    Escape::None
                };
                return Typed::Nothing;
            }
            Escape::Sequence => {
                if (0x40..=0x7e).contains(&byte) {
                    self.escape = Escape::None;
                }
                return Typed::Nothing;
            }
            Escape::None => {}
        }
        let after_cr = std::mem::take(&mut self.after_cr);
        match byte {
            0x1b => {
                self.escape = Escape::Start;
                Typed::Nothing
            }
            b'\r' => {
                self.after_cr = true;
                self.submit()
            }
            b'\n' if after_cr => Typed::Nothing,
            b'\n' => self.submit(),
            0x7f | 0x08 => {
                if self.line.pop().is_some() {
                    Typed::Echo(b"\x08 \x08".to_vec())
                } else {
                    Typed::Nothing
                }
            }
            0x03 => Typed::Quit,
            0x04 if self.line.is_empty() => Typed::Quit,
            0x20..=0x7e | 0x80..=0xff => {
                if self.line.len() < MAX_LINE {
                    self.line.push(byte);
                    Typed::Echo(vec![byte])
                } else {
                    Typed::Nothing
                }
            }
            _ => Typed::Nothing,
        }
    }

    fn submit(&mut self) -> Typed {
        Typed::Submit(String::from_utf8_lossy(&std::mem::take(&mut self.line)).into_owned())
    }
}

/// Terminal text from the world: control characters are dropped, so text from
/// one participant cannot drive another participant's terminal.
fn terminal_text(data: &str) -> String {
    data.chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect::<String>()
        .replace('\n', "\r\n")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::world::{WorldCore, WorldHandle, WorldLimits};
    use crate::world_session::WorldService;
    use russh::{ChannelMsg, client, keys::PrivateKeyWithHashAlg};

    struct TrustAll;

    impl client::Handler for TrustAll {
        type Error = russh::Error;

        async fn check_server_key(
            &mut self,
            _key: &russh::keys::PublicKeyOrCertificate,
        ) -> Result<bool, russh::Error> {
            Ok(true)
        }
    }

    async fn start(hub: WorldHub) -> (u16, tokio::task::JoinHandle<std::io::Result<()>>) {
        start_limited(hub, test_store(), MAX_CONNECTIONS).await
    }

    async fn start_limited(
        hub: WorldHub,
        blobs: iroh_blobs::api::Store,
        max_connections: usize,
    ) -> (u16, tokio::task::JoinHandle<std::io::Result<()>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        (
            port,
            tokio::spawn(serve_limited(
                listener,
                hub,
                host_key,
                blobs,
                max_connections,
            )),
        )
    }

    async fn start_with_store(
        hub: WorldHub,
        blobs: iroh_blobs::api::Store,
    ) -> (u16, tokio::task::JoinHandle<std::io::Result<()>>) {
        start_limited(hub, blobs, MAX_CONNECTIONS).await
    }

    fn test_store() -> iroh_blobs::api::Store {
        iroh_blobs::store::mem::MemStore::new().into()
    }

    async fn connect(port: u16, password: &str) -> client::Handle<TrustAll> {
        connect_as(port, "", password).await
    }

    async fn connect_as(port: u16, user: &str, password: &str) -> client::Handle<TrustAll> {
        let config = Arc::new(client::Config::default());
        let mut session = client::connect(config, ("127.0.0.1", port), TrustAll)
            .await
            .unwrap();
        assert!(
            session
                .authenticate_password(user, password)
                .await
                .unwrap()
                .success()
        );
        session
    }

    async fn explorer(session: &client::Handle<TrustAll>) -> Channel<client::Msg> {
        let channel = session.channel_open_session().await.unwrap();
        channel.request_shell(true).await.unwrap();
        channel
    }

    async fn read_until_prompt(channel: &mut Channel<client::Msg>) -> String {
        let mut text = String::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !text.ends_with(PROMPT) {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => {
                        text.push_str(&String::from_utf8_lossy(&data));
                    }
                    Some(_) => {}
                    None => panic!("explorer closed; got {text:?}"),
                }
            }
        })
        .await
        .expect("timed out waiting for the explorer prompt");
        text
    }

    async fn command(channel: &mut Channel<client::Msg>, line: &str) -> String {
        channel.data(format!("{line}\r").as_bytes()).await.unwrap();
        read_until_prompt(channel).await
    }

    async fn quit(channel: &mut Channel<client::Msg>) -> String {
        channel.data(&b"quit\r"[..]).await.unwrap();
        let mut text = String::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(message) = channel.wait().await {
                if let ChannelMsg::Data { data } = message {
                    text.push_str(&String::from_utf8_lossy(&data));
                }
            }
        })
        .await
        .expect("explorer did not close after quit");
        text
    }

    fn account_id(credential: &str) -> String {
        credential.split('.').nth(1).unwrap().to_owned()
    }

    fn credential_in(text: &str) -> String {
        text.split("credential (shown once): ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .unwrap()
            .to_owned()
    }

    fn group_id(text: &str, name: &str) -> String {
        text.lines()
            .find(|line| line.contains(name) && line.contains("you:"))
            .and_then(|line| line.split_whitespace().next())
            .unwrap()
            .to_owned()
    }

    #[tokio::test]
    async fn the_explorer_shows_the_directory_as_text_to_the_admin_and_anonymous() {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let signed = world
            .directory(
                Caller::default(),
                DirectoryAction::SignUp {
                    name: "Ada".to_owned(),
                },
                false,
            )
            .await
            .unwrap();
        let ada = signed.view.viewer.clone();
        let hub = WorldHub::single(WorldService::new(world, Some("admin".to_owned())));
        let (port, server) = start(hub).await;

        for (password, is_admin) in [("admin", true), ("", false)] {
            let session = connect_as(port, "explore:", password).await;
            let mut channel = explorer(&session).await;
            let text = read_until_prompt(&mut channel).await;
            assert!(text.contains(&ada) && text.contains("Ada"), "{text}");
            assert_eq!(text.contains("viewer: admin"), is_admin, "{text}");
            assert!(quit(&mut channel).await.contains("bye"));
        }
        server.abort();
    }

    #[tokio::test]
    async fn an_explorer_signs_up_and_reads_help_and_refuses_bad_lines() {
        let (port, server) = start(lobby_hub()).await;
        let session = connect_as(port, "explore:", "").await;
        let mut channel = explorer(&session).await;
        read_until_prompt(&mut channel).await;

        let text = command(&mut channel, "signup Cy").await;
        assert!(text.contains("credential (shown once): acct."), "{text}");
        assert!(text.contains("viewer: ") && text.contains("Cy"), "{text}");
        let text = command(&mut channel, "bogus").await;
        assert!(text.contains("error: unknown command bogus"), "{text}");
        let text = command(&mut channel, "group new").await;
        assert!(text.contains("error:"), "{text}");
        let text = command(&mut channel, "help").await;
        assert!(text.contains("commands:"), "{text}");
        let text = command(&mut channel, "group new - Club | Members only").await;
        assert!(
            text.contains("Club") && text.contains("Members only"),
            "{text}"
        );
        assert!(text.contains("you: admin"), "{text}");
        quit(&mut channel).await;
        server.abort();
    }

    #[tokio::test]
    async fn explorers_manage_friends_and_group_members_over_ssh() {
        let (port, server) = start(lobby_hub()).await;
        let ada_session = connect_as(port, "explore:", "").await;
        let mut ada = explorer(&ada_session).await;
        read_until_prompt(&mut ada).await;
        let ada_credential = credential_in(&command(&mut ada, "signup Ada").await);
        let ada_id = account_id(&ada_credential);
        let text = command(&mut ada, "group new - Club | Members only").await;
        let club = group_id(&text, "Club");

        let bo_session = connect_as(port, "explore:", "").await;
        let mut bo = explorer(&bo_session).await;
        read_until_prompt(&mut bo).await;
        let bo_credential = credential_in(&command(&mut bo, "signup Bo").await);
        let bo_id = account_id(&bo_credential);

        let text = command(&mut ada, &format!("friend request {bo_id}")).await;
        assert!(text.contains(&format!("requests out: {bo_id}")), "{text}");
        let text = command(&mut bo, &format!("friend accept {ada_id}")).await;
        assert!(text.contains(&format!("friends: {ada_id}")), "{text}");
        let text = command(&mut ada, &format!("friend remove {bo_id}")).await;
        assert!(!text.contains(&format!("friends: {bo_id}")), "{text}");

        let text = command(
            &mut ada,
            &format!("group member {club} account:{bo_id} read"),
        )
        .await;
        assert!(text.contains(&format!("account {bo_id}")), "{text}");
        let text = command(&mut bo, "view").await;
        assert!(
            text.contains("Club") && text.contains("you: read"),
            "{text}"
        );
        let text = command(&mut ada, "verify nobody").await;
        assert!(text.contains("error:"), "{text}");
        server.abort();
    }

    #[tokio::test]
    async fn a_key_login_signs_up_with_its_key_and_is_recognised_again() {
        let (port, server) = start(lobby_hub()).await;
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let hex = hex_encode(&key.public_key().key_data().ed25519().unwrap().0);

        let session = connect_key(port, "explore:", &key).await;
        let mut channel = explorer(&session).await;
        read_until_prompt(&mut channel).await;
        let text = command(&mut channel, "keysignup Kay").await;
        assert!(text.contains(&format!("viewer: {hex}")), "{text}");
        quit(&mut channel).await;

        let again = connect_key(port, "explore:", &key).await;
        let mut channel = explorer(&again).await;
        let text = read_until_prompt(&mut channel).await;
        assert!(
            text.contains(&format!("viewer: {hex}")) && text.contains("Kay"),
            "{text}"
        );

        let stranger = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let session = connect_key(port, "explore:", &stranger).await;
        let mut channel = explorer(&session).await;
        let text = read_until_prompt(&mut channel).await;
        assert!(text.contains("viewer: anonymous"), "{text}");
        server.abort();
    }

    async fn connect_key(port: u16, user: &str, key: &PrivateKey) -> client::Handle<TrustAll> {
        let config = Arc::new(client::Config::default());
        let mut session = client::connect(config, ("127.0.0.1", port), TrustAll)
            .await
            .unwrap();
        let result = session
            .authenticate_publickey(
                user,
                PrivateKeyWithHashAlg::new(Arc::new(key.clone()), None),
            )
            .await
            .unwrap();
        assert!(result.success());
        session
    }

    #[test]
    fn typing_edits_a_line_drops_escape_sequences_and_submits_once_per_enter() {
        let mut typing = Typing::default();
        let mut submitted = Vec::new();
        for byte in b"ab\x7fc\x1b[A\r\n" {
            if let Typed::Submit(line) = typing.feed(*byte) {
                submitted.push(line);
            }
        }
        assert_eq!(submitted, vec!["ac".to_owned()]);
        assert!(matches!(typing.feed(3), Typed::Quit));
    }

    async fn shell(session: &client::Handle<TrustAll>) -> Channel<client::Msg> {
        let channel = session.channel_open_session().await.unwrap();
        channel
            .request_pty(true, "xterm", 80, 24, 0, 0, &[])
            .await
            .unwrap();
        channel.request_shell(true).await.unwrap();
        channel
    }

    async fn read_until(
        channel: &mut Channel<client::Msg>,
        terminal: &mut vt100::Parser,
        needle: &str,
    ) {
        let _ = read_until_bytes(channel, terminal, needle).await;
    }

    async fn read_until_bytes(
        channel: &mut Channel<client::Msg>,
        terminal: &mut vt100::Parser,
        needle: &str,
    ) -> Vec<u8> {
        let mut received = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !terminal.screen().contents().contains(needle) {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => {
                        terminal.process(&data);
                        received.extend_from_slice(&data);
                    }
                    Some(_) => {}
                    None => panic!("shell closed before {needle:?}"),
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for {needle:?}; screen: {:?}",
                terminal.screen().contents()
            )
        });
        received
    }

    fn lobby_hub() -> WorldHub {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        WorldHub::single(WorldService::new(world, Some("admin".to_owned())))
    }

    fn sequence_count(bytes: &[u8], sequence: &str) -> usize {
        bytes
            .windows(sequence.len())
            .filter(|window| *window == sequence.as_bytes())
            .count()
    }

    fn sequence_position(bytes: &[u8], sequence: &str) -> Option<usize> {
        bytes
            .windows(sequence.len())
            .position(|window| window == sequence.as_bytes())
    }

    #[tokio::test]
    async fn a_raw_shell_enters_and_leaves_the_alternate_screen() {
        let hub = lobby_hub();
        let invite = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .invite(Some("admin"), "ssh-raw-alt".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start(hub).await;
        let session = connect(port, &capability).await;
        let mut channel = shell(&session).await;

        // The enter sequence must be the very first bytes on the channel:
        // a live, accepted session enters the alternate screen before its
        // first frame, not at teardown.
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while sequence_count(&bytes, ALT_SCREEN_ENTER) == 0 {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => bytes.extend_from_slice(&data),
                    Some(_) => {}
                    None => break,
                }
            }
        })
        .await
        .expect("timed out waiting for the alternate screen to open");
        assert_eq!(
            sequence_position(&bytes, ALT_SCREEN_ENTER),
            Some(0),
            "enter sequence was not the first output: {bytes:?}"
        );
        assert_eq!(sequence_count(&bytes, ALT_SCREEN_LEAVE), 0);

        drop(channel);
        drop(session);
        server.abort();
    }

    #[tokio::test]
    async fn a_refused_raw_shell_leaves_the_alternate_screen_and_repeats_its_error() {
        let (port, server) = start(lobby_hub()).await;
        let session = connect(port, &"0".repeat(64)).await;
        let mut channel = shell(&session).await;

        // A refused join ends the session server-side: the server writes the
        // leave sequence and closes the channel, so read until the close.
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => bytes.extend_from_slice(&data),
                    Some(_) => {}
                    None => break,
                }
            }
        })
        .await
        .expect("timed out waiting for the refused shell to close");

        assert_eq!(
            sequence_position(&bytes, ALT_SCREEN_ENTER),
            Some(0),
            "enter sequence was not the first output: {bytes:?}"
        );
        assert_eq!(
            sequence_count(&bytes, ALT_SCREEN_ENTER),
            1,
            "enter sequence must be written exactly once: {bytes:?}"
        );
        assert_eq!(
            sequence_count(&bytes, ALT_SCREEN_LEAVE),
            1,
            "leave sequence must be written exactly once: {bytes:?}"
        );
        let leave_at = sequence_position(&bytes, ALT_SCREEN_LEAVE).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let error_after_leave = text.rfind("join denied").expect("join error missing");
        assert!(
            error_after_leave > leave_at,
            "the last error must be re-sent after the leave sequence: {text:?}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_command_shell_stays_on_the_primary_screen() {
        let hub = lobby_hub();
        let invite = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .invite(Some("admin"), "ssh-cmd-alt".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start(hub).await;
        let session = connect_as(port, "cmd:", &capability).await;
        let mut channel = shell(&session).await;
        let mut text = String::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while text.len() < 32 {
                if let Some(ChannelMsg::Data { data }) = channel.wait().await {
                    text.push_str(&String::from_utf8_lossy(&data));
                }
            }
        })
        .await
        .expect("timed out waiting for the command prompt");
        assert!(
            !text
                .as_bytes()
                .windows(ALT_SCREEN_ENTER.len())
                .any(|window| window == ALT_SCREEN_ENTER.as_bytes())
        );
        assert!(
            !text
                .as_bytes()
                .windows(ALT_SCREEN_LEAVE.len())
                .any(|window| window == ALT_SCREEN_LEAVE.as_bytes())
        );
        server.abort();
    }

    #[test]
    fn dropped_input_notice_counts_and_quiet_zero() {
        assert_eq!(dropped_input_notice(0), None);
        // Below the threshold the viewer is not told yet: the drops stay
        // counted until they add up to a sustained loss.
        assert_eq!(dropped_input_notice(INPUT_DROP_NOTICE_BYTES - 1), None);
        let notice = dropped_input_notice(INPUT_DROP_NOTICE_BYTES).unwrap();
        assert!(notice.contains("1024"));
        assert!(notice.contains("dropped"));
    }

    #[test]
    fn dropped_input_notice_follows_the_shared_counter() {
        // Drive the counter exactly the way the russh handler does when the
        // queue refuses input mid-session: the shell must see those drops,
        // not the zero snapshot a copy taken at shell-open would hold.
        let counter = AtomicUsize::new(0);
        record_dropped_input(&counter, 600);
        assert_eq!(claim_dropped_input_notice(&counter), None);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            600,
            "a below-threshold claim must leave the drops counted"
        );
        record_dropped_input(&counter, 600);
        let (bytes, notice) = claim_dropped_input_notice(&counter).unwrap();
        assert_eq!(bytes, 1200);
        assert!(notice.contains("1200"));
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        // The claim was the reset: nothing is reported twice.
        assert_eq!(claim_dropped_input_notice(&counter), None);
        // Drops counted after a claim are reported on their own.
        record_dropped_input(&counter, INPUT_DROP_NOTICE_BYTES);
        let (bytes, _) = claim_dropped_input_notice(&counter).unwrap();
        assert_eq!(bytes, INPUT_DROP_NOTICE_BYTES);
        // A notice that cannot be handed to the viewer restores its claim
        // (the send path adds the bytes back), so the same drops are
        // reported by the next successful claim instead of being lost.
        record_dropped_input(&counter, INPUT_DROP_NOTICE_BYTES);
        let (bytes, _) = claim_dropped_input_notice(&counter).unwrap();
        counter.fetch_add(bytes, Ordering::Relaxed);
        let (restored, _) = claim_dropped_input_notice(&counter).unwrap();
        assert_eq!(restored, INPUT_DROP_NOTICE_BYTES);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn explorer_notice_prepends_to_next_output_and_stays_quiet_below_threshold() {
        // Drive the shared counter exactly the way the russh handler does
        // for an explorer session whose input queue is full (#723).
        let counter = AtomicUsize::new(0);
        record_dropped_input(&counter, 600);
        let mut out = b"id> ".to_vec();
        assert_eq!(prepend_dropped_input_notice(&counter, &mut out), None);
        assert_eq!(out, b"id> ", "below-threshold drops stay quiet");
        assert_eq!(counter.load(Ordering::Relaxed), 600);

        record_dropped_input(&counter, 600);
        let mut out = b"id> ".to_vec();
        let claimed = prepend_dropped_input_notice(&counter, &mut out).unwrap();
        assert_eq!(claimed, 1200);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("[-- input dropped: 1200 bytes"), "{text}");
        assert!(text.ends_with("id> "), "notice precedes the next output");
        assert_eq!(counter.load(Ordering::Relaxed), 0);

        // A failed write restores the claim (run_explorer adds it back),
        // so the same drops are prepended again rather than lost.
        record_dropped_input(&counter, INPUT_DROP_NOTICE_BYTES);
        let mut out = Vec::new();
        let claimed = prepend_dropped_input_notice(&counter, &mut out).unwrap();
        counter.fetch_add(claimed, Ordering::Relaxed);
        let mut out = Vec::new();
        assert_eq!(
            prepend_dropped_input_notice(&counter, &mut out),
            Some(INPUT_DROP_NOTICE_BYTES)
        );
        assert!(!out.is_empty(), "notice alone is output worth sending");
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused_inside_the_shell() {
        let (port, server) = start(lobby_hub()).await;

        let session = connect(port, &"0".repeat(64)).await;
        let mut channel = shell(&session).await;
        let mut terminal = vt100::Parser::new(24, 80, 0);
        read_until(&mut channel, &mut terminal, "join denied").await;
        server.abort();
    }

    #[tokio::test]
    async fn a_shell_accepts_chat_and_reads_authorized_records_pages() {
        let hub = lobby_hub();
        let world = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .world()
            .clone();
        let mut events = world.subscribe();
        let invite = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .invite(Some("admin"), "ssh-chat".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start(hub).await;
        let session = connect_as(port, "cmd:", &capability).await;
        let mut channel = shell(&session).await;
        let mut terminal = vt100::Parser::new(24, 80, 0);
        read_until(&mut channel, &mut terminal, "world>").await;

        channel.data(&b"hello from SSH\r"[..]).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(event) = events.recv().await {
                    if matches!(event.kind, crate::world::WorldEventKind::Chat { .. }) {
                        break;
                    }
                }
            }
        })
        .await
        .expect("chat line was not sent to the world");
        channel.data(&b"/records\r"[..]).await.unwrap();
        read_until(&mut channel, &mut terminal, "records at sequence").await;
        server.abort();
    }

    #[tokio::test]
    async fn connections_over_the_limit_are_refused_at_auth() {
        let (port, server) = start_limited(lobby_hub(), test_store(), 1).await;

        let _first = connect(port, &"0".repeat(64)).await;
        let config = Arc::new(client::Config::default());
        let mut second = client::connect(config, ("127.0.0.1", port), TrustAll)
            .await
            .unwrap();
        assert!(
            !second
                .authenticate_password("", &"0".repeat(64))
                .await
                .unwrap()
                .success()
        );
        server.abort();
    }

    #[test]
    fn ssh_targets_parse_user_host_and_port() {
        let parse = |text: &str| text.parse::<SshTarget>();
        let target = |user: &str, host: &str, port| SshTarget {
            user: user.to_owned(),
            host: host.to_owned(),
            port,
        };
        assert_eq!(
            parse("example.org").unwrap(),
            target("id", "example.org", 22)
        );
        assert_eq!(
            parse("ada@example.org:2222").unwrap(),
            target("ada", "example.org", 2222)
        );
        assert_eq!(parse("[::1]:2022").unwrap(), target("id", "::1", 2022));
        assert_eq!(parse("me@[::1]").unwrap(), target("me", "::1", 22));
        for bad in ["", "host:", "host:99999", "::1", "[::1", "@host", "[::1]x"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_exec_command_is_the_contracts_ssh_transport() {
        let contract = crate::shell_contract::ShellContract::embedded().unwrap();
        assert_eq!(
            contract.transports["ssh-exec"]["command"],
            WORLD_EXEC_COMMAND
        );
        assert_eq!(
            contract.transports["ssh-exec"]["envelope"],
            contract.transports["iroh"]["envelope"]
        );
    }

    /// A program whose view is the same for every viewer and every connection.
    struct Shown;

    impl crate::world::WorldProgram for Shown {
        fn view(&mut self, _viewer: &crate::world::Participant) -> anyhow::Result<Option<String>> {
            Ok(Some("shown".to_owned()))
        }
    }

    /// One world served over iroh and SSH at once.
    struct TwoTransports {
        world: WorldHandle,
        capability: String,
        port: u16,
        fingerprint: String,
        endpoint: iroh::Endpoint,
        addr: iroh::EndpointAddr,
        _router: iroh::protocol::Router,
        ssh: tokio::task::JoinHandle<std::io::Result<()>>,
    }

    async fn two_transports() -> TwoTransports {
        use iroh::endpoint::{RelayMode, presets};

        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        world
            .install_program("ab".repeat(32), 5, Box::new(Shown))
            .await
            .unwrap();
        let (_, token) = world
            .issue("ada", crate::world::WorldScopes::GUEST)
            .await
            .unwrap();
        let hub = WorldHub::single(WorldService::new(world.clone(), Some("admin".to_owned())));

        let bind = || async {
            iroh::Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .bind()
                .await
                .unwrap()
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let fingerprint = host_key_fingerprint(&host_key);
        let host = bind().await;
        let router = iroh::protocol::Router::builder(host.clone())
            .accept(
                crate::world_net::WORLD_ALPN,
                crate::world_net::WorldProtocol::new(hub.clone()),
            )
            .accept(SSH_TUNNEL_ALPN, SshTunnelProtocol::with_port(port))
            .spawn();
        let addr = iroh::EndpointAddr::from_parts(
            host.id(),
            host.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(SocketAddr::new(ip, a.port()))
            }),
        );

        let ssh = tokio::spawn(serve_limited(
            listener,
            hub,
            host_key,
            test_store(),
            MAX_CONNECTIONS,
        ));
        TwoTransports {
            world,
            capability: token.expose().to_owned(),
            port,
            fingerprint,
            endpoint: bind().await,
            addr,
            _router: router,
            ssh,
        }
    }

    impl TwoTransports {
        fn target(&self) -> SshTarget {
            SshTarget {
                user: "id".to_owned(),
                host: "127.0.0.1".to_owned(),
                port: self.port,
            }
        }

        fn trust(&self) -> HostKeyCheck {
            HostKeyCheck::Fingerprint(self.fingerprint.clone())
        }
    }

    /// The raw frame bodies a transport delivers, read in their own task so
    /// a test can wait for quiet without cancelling a read.
    struct RawFrames {
        frames: mpsc::UnboundedReceiver<Vec<u8>>,
    }

    impl RawFrames {
        fn read<R: tokio::io::AsyncRead + Unpin + Send + 'static>(mut recv: R) -> Self {
            let (tx, frames) = mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Ok(Some(body)) = crate::frames::read_frame_from(&mut recv).await {
                    if tx.send(body).is_err() {
                        break;
                    }
                }
            });
            Self { frames }
        }

        /// Every frame until the host has been quiet for a moment.
        async fn drain(&mut self) -> Vec<Vec<u8>> {
            let mut frames = Vec::new();
            while let Ok(Some(frame)) =
                tokio::time::timeout(Duration::from_millis(400), self.frames.recv()).await
            {
                frames.push(frame);
            }
            frames
        }
    }

    fn texts(frames: &[Vec<u8>]) -> Vec<String> {
        frames
            .iter()
            .map(|frame| String::from_utf8_lossy(frame).into_owned())
            .collect()
    }

    /// F13.2's exit test: one scripted session over iroh and over an SSH exec
    /// channel, in lockstep on one world, unwraps to the same frame bytes.
    #[tokio::test]
    async fn an_exec_channel_carries_the_same_frames_as_the_iroh_stream() {
        use crate::frames::{write_frame, write_frame_to};

        let host = two_transports().await;
        let iroh_client = crate::world_net::WorldClient::connect(&host.endpoint, host.addr.clone())
            .await
            .unwrap();
        let (conn, mut iroh_send, iroh_recv) = iroh_client.into_parts();
        let mut iroh_frames = RawFrames::read(iroh_recv);
        let (_connection, stream) = connect_frames(&host.target(), host.trust()).await.unwrap();
        let (ssh_recv, mut ssh_send) = tokio::io::split(stream);
        let mut ssh_frames = RawFrames::read(ssh_recv);

        let cap = &host.capability;
        let join = crate::world_net::route_frame(
            &serde_json::json!({"type": "join", "capability": cap, "after": null}),
            Some("lobby"),
        );
        write_frame(&mut iroh_send, join.as_bytes()).await.unwrap();
        let iroh_joined = iroh_frames.drain().await;
        write_frame_to(&mut ssh_send, join.as_bytes())
            .await
            .unwrap();
        let ssh_joined = ssh_frames.drain().await;
        assert_eq!(texts(&iroh_joined), texts(&ssh_joined));
        assert!(
            iroh_frames.drain().await.is_empty(),
            "a second join is invisible"
        );

        // (who sends, frame): broadcasts reach both; replies go to whoever asked.
        let script = [
            (
                Some(true),
                serde_json::json!({"type": "chat", "text": "over iroh"}),
            ),
            (
                Some(false),
                serde_json::json!({"type": "input", "data_hex": "6869"}),
            ),
            (None, serde_json::json!({"type": "info", "capability": cap})),
            (
                None,
                serde_json::json!({"type": "records", "capability": cap}),
            ),
            (None, serde_json::json!({"type": "caps", "capability": cap})),
            (
                None,
                serde_json::json!({"type": "resize", "width": 40, "height": 10}),
            ),
            (
                None,
                serde_json::json!({"type": "chat", "text": "\u{1}bad"}),
            ),
            (None, serde_json::json!({"type": "teleport"})),
        ];
        let mut checked = 0;
        for (sender, frame) in script {
            let text = frame.to_string();
            if sender != Some(false) {
                write_frame(&mut iroh_send, text.as_bytes()).await.unwrap();
            }
            if sender != Some(true) {
                write_frame_to(&mut ssh_send, text.as_bytes())
                    .await
                    .unwrap();
            }
            let over_iroh = iroh_frames.drain().await;
            let over_ssh = ssh_frames.drain().await;
            assert!(!over_iroh.is_empty(), "{text}");
            assert_eq!(over_iroh, over_ssh, "{text}: {:?}", texts(&over_iroh));
            checked += over_iroh.len();
        }
        assert!(checked >= 10, "{checked}");
        // Only the chat and the input were journaled: the frames themselves
        // never touch it.
        let token = crate::world::JoinCapability::from_wire(cap.clone());
        let snapshot = host.world.snapshot(token.clone()).await.unwrap();
        // The install, the chat, the input, and each side's sanitized chat.
        assert_eq!(snapshot.current_sequence, 5);
        drop(ssh_send);
        drop(ssh_frames);
        conn.close(0_u32.into(), b"done");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let after = host.world.snapshot(token).await.unwrap();
        assert_eq!(after.current_sequence, snapshot.current_sequence);
        host.ssh.abort();
    }

    #[tokio::test]
    async fn the_exec_transport_refuses_other_commands_untrusted_keys_and_oversized_frames() {
        let host = two_transports().await;

        // An unknown key is refused, and the error names its fingerprint.
        let Err(error) = connect_frames(
            &host.target(),
            HostKeyCheck::Fingerprint("SHA256:not-the-key".to_owned()),
        )
        .await
        else {
            panic!("an untrusted key was accepted");
        };
        let error = error.to_string();
        assert!(error.contains(&host.fingerprint), "{error}");
        let empty = tempfile::NamedTempFile::new().unwrap();
        assert!(
            connect_frames(
                &host.target(),
                HostKeyCheck::KnownHosts(empty.path().to_owned())
            )
            .await
            .is_err()
        );

        // Any other exec command, and a second session on one connection, fail.
        let session = connect(host.port, "").await;
        let mut channel = session.channel_open_session().await.unwrap();
        channel.exec(true, "sh -c id").await.unwrap();
        assert!(matches!(
            channel.wait().await,
            Some(russh::ChannelMsg::Failure)
        ));
        let mut channel = session.channel_open_session().await.unwrap();
        channel.exec(true, WORLD_EXEC_COMMAND).await.unwrap();
        assert!(matches!(
            channel.wait().await,
            Some(russh::ChannelMsg::Success)
        ));
        let mut second = session.channel_open_session().await.unwrap();
        second.exec(true, WORLD_EXEC_COMMAND).await.unwrap();
        assert!(matches!(
            second.wait().await,
            Some(russh::ChannelMsg::Failure)
        ));

        // A length over the frame limit ends the session unread, as on iroh.
        let too_long = u32::try_from(crate::frames::MAX_FRAME_BYTES + 1)
            .unwrap()
            .to_be_bytes();
        channel.data(&too_long[..]).await.unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match channel.wait().await {
                    Some(russh::ChannelMsg::Eof | russh::ChannelMsg::Close) | None => break,
                    Some(_) => {}
                }
            }
        })
        .await;
        assert!(ended.is_ok(), "the session did not end");

        // A login grants nothing on the frame channel: a bad capability is refused.
        let (_connection, stream) = connect_frames(&host.target(), host.trust()).await.unwrap();
        let (recv, mut send) = tokio::io::split(stream);
        let mut frames = RawFrames::read(recv);
        crate::frames::write_frame_to(&mut send, br#"{"type":"join","capability":"1.forged"}"#)
            .await
            .unwrap();
        let reply = frames.drain().await;
        assert_eq!(
            texts(&reply),
            [r#"{"type":"error","message":"join denied"}"#]
        );
        host.ssh.abort();
    }

    /// F13.3: an SSH client reaches the world SSH server through iroh only —
    /// no TCP route to it is used after the tunnel connects.
    #[tokio::test]
    async fn ssh_reaches_the_world_through_the_iroh_tunnel() {
        let host = two_transports().await;
        let conn = crate::connectivity::connect(&host.endpoint, host.addr.clone(), SSH_TUNNEL_ALPN)
            .await
            .unwrap();
        let (send, recv) = conn.open_bi().await.unwrap();
        let stream = tokio::io::join(recv, send);
        let config = Arc::new(client::Config::default());
        let mut session = client::connect_stream(config, stream, TrustAll)
            .await
            .unwrap();
        assert!(
            session
                .authenticate_password("", host.capability.as_str())
                .await
                .unwrap()
                .success()
        );
        // Inside the tunnel the frame transport works as usual.
        let mut channel = session.channel_open_session().await.unwrap();
        channel.exec(true, WORLD_EXEC_COMMAND).await.unwrap();
        assert!(matches!(
            channel.wait().await,
            Some(russh::ChannelMsg::Success)
        ));
        let join = crate::world_net::route_frame(
            &serde_json::json!({"type": "join", "capability": host.capability, "after": null}),
            Some("lobby"),
        );
        channel
            .data(
                crate::frames::encode_frame(join.as_bytes())
                    .unwrap()
                    .as_slice(),
            )
            .await
            .unwrap();
        let mut buffer = crate::frames::FrameBuffer::default();
        let frame = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match channel.wait().await {
                    Some(russh::ChannelMsg::Data { data }) => {
                        buffer.push(&data);
                        if let Some(crate::frames::Decoded::Frame(body)) = buffer.next_frame() {
                            break String::from_utf8(body).unwrap();
                        }
                    }
                    other => panic!("tunnel closed early: {other:?}"),
                }
            }
        })
        .await
        .unwrap();
        assert!(frame.contains("\"type\":\"snapshot\""), "{frame}");
        conn.close(0_u32.into(), b"done");
        host.ssh.abort();
    }

    #[test]
    fn terminal_shell_frames_conform_to_the_shell_contract() {
        use crate::shell_contract::{Phase, ShellContract};
        let contract = ShellContract::embedded().unwrap();
        let check = |frame: &str, phase| {
            let value: serde_json::Value = serde_json::from_str(frame).unwrap();
            contract
                .check_client_frame(&value, phase)
                .map(str::to_owned)
                .unwrap_or_else(|error| panic!("{frame}: {error}"))
        };
        for (world, command_mode) in [(None, false), (Some("arena"), true)] {
            let login = Login {
                world: world.map(str::to_owned),
                capability: "cap".to_owned(),
                explore: false,
                command_mode,
                key: None,
            };
            assert_eq!(check(&join_frame(&login), Phase::Opening), "join");
        }
        for input in [
            SshInputFrame::Bytes(b"\x1b[A".to_vec()),
            SshInputFrame::Resize(80, 24),
            SshInputFrame::Resize(0, 0),
            SshInputFrame::Resize(4000, 1000),
        ] {
            check(&raw_frame(&input), Phase::Joined);
        }
        let mut sent = Vec::new();
        for line in [
            "hello",
            "/input 00ff",
            "/records",
            "/records key-",
            "/stats",
            "/caps",
            "/logs",
            "/help",
            "/documents",
            "/document a.md",
            "/nope",
            "/quit",
        ] {
            let command = world_command(line).unwrap();
            if let Some(frame) = command_frame(&command, "cap") {
                sent.push(check(&frame, Phase::Joined));
            }
        }
        let more = WorldCommand::RecordsContinue {
            prefix: None,
            after: "k".to_owned(),
        };
        sent.push(check(&command_frame(&more, "cap").unwrap(), Phase::Joined));
        assert_eq!(
            sent,
            [
                "chat", "input", "records", "records", "info", "caps", "records"
            ]
        );
    }

    #[test]
    fn terminal_text_drops_control_characters_but_keeps_lines() {
        assert_eq!(
            terminal_text("a\x1b]52;c;AAAA\x07b\r\nc\td"),
            "a]52;c;AAAAb\r\nc\td"
        );
    }

    #[test]
    fn world_line_buffer_handles_fragmented_packets_and_crlf_once() {
        let mut input = WorldLineBuffer::default();
        assert!(input.push(b"hel").is_empty());
        assert_eq!(
            input.push(b"lo\r"),
            vec![WorldCommand::Chat("hello".into())]
        );
        assert!(input.push(b"\n").is_empty());
        assert_eq!(
            input.push(b"next\n"),
            vec![WorldCommand::Chat("next".into())]
        );
    }

    #[test]
    fn world_lines_map_commands_without_echoing_or_treating_empty_as_chat() {
        assert_eq!(world_command(""), None);
        assert_eq!(
            world_command("hi there"),
            Some(WorldCommand::Chat("hi there".into()))
        );
        assert_eq!(
            world_command("/input 00aF"),
            Some(WorldCommand::Input("00af".into()))
        );
        assert_eq!(
            world_command("/records abc"),
            Some(WorldCommand::Records {
                prefix: Some("abc".into())
            })
        );
        assert_eq!(
            world_command("/documents notes"),
            Some(WorldCommand::Documents {
                prefix: Some("notes".into())
            })
        );
        assert_eq!(
            world_command("/document notes.md"),
            Some(WorldCommand::Document("notes.md".into()))
        );
        assert_eq!(world_command("/help"), Some(WorldCommand::Help));
        assert_eq!(world_command("/quit"), Some(WorldCommand::Quit));
        assert_eq!(world_command("/stats"), Some(WorldCommand::Stats));
        assert_eq!(world_command("/caps"), Some(WorldCommand::Caps));
        assert_eq!(
            world_command("/logs"),
            Some(WorldCommand::Logs { seconds: None })
        );
        assert_eq!(
            world_command("/logs 5"),
            Some(WorldCommand::Logs { seconds: Some(5) })
        );
        assert_eq!(
            world_command("/input nope"),
            Some(WorldCommand::Message("usage: /input <hex>".into()))
        );
    }

    #[test]
    fn world_line_buffer_discards_overlong_line_until_newline() {
        let mut input = WorldLineBuffer::default();
        assert!(input.push(&vec![b'x'; MAX_LINE + 1]).is_empty());
        assert_eq!(
            input.push(b"tail\nokay\n"),
            vec![WorldCommand::Chat("okay".into())]
        );
    }

    #[test]
    fn world_line_buffer_drops_fragmented_terminal_control_strings() {
        let mut input = WorldLineBuffer::default();
        assert!(input.push(b"safe\x1b]52;c;secret").is_empty());
        assert_eq!(
            input.push(b"\x07still safe\r\n"),
            vec![WorldCommand::Chat("safestill safe".into())]
        );
    }

    #[test]
    fn records_render_as_bounded_json_safe_terminal_text() {
        let records: crate::world::Records =
            serde_json::from_str(r#"{"a\u001b[31m":{"n":1},"b":"line\nvalue"}"#).unwrap();
        let text = render_records(&records, 7, Some("b"));
        assert!(text.contains("sequence 7"));
        assert!(text.contains("\"a\\u001b[31m\" = {\"n\":1}"));
        assert!(text.contains("\"b\" = \"line\\nvalue\""));
        assert!(text.contains("more records; fetching next page"));
        assert!(text.len() <= MAX_RECORDS_TERMINAL_BYTES);
    }

    /// The `/records` drain is bounded in total, not just per page: the per-page
    /// cap is [`MAX_RECORDS_TERMINAL_BYTES`], but a world with many records must
    /// also stop after [`MAX_RECORDS_TOTAL_BYTES`] across pages.
    #[test]
    fn the_records_drain_is_bounded_in_total_as_well_as_per_page() {
        // A single page is truncated at the per-page cap, so a page never
        // exceeds it...
        let one_page = render_records(&records_of(4096), 1, None);
        assert!(
            one_page.len() <= MAX_RECORDS_TERMINAL_BYTES + 64,
            "one page stays under the per-page cap: {}",
            one_page.len()
        );
        // ...and the whole drain has a larger, but still finite, total cap, so
        // a many-page world stops instead of streaming without bound.
        const {
            assert!(
                MAX_RECORDS_TOTAL_BYTES > MAX_RECORDS_TERMINAL_BYTES,
                "the total cap must exceed the per-page cap"
            )
        };
        const {
            assert!(
                MAX_RECORDS_TOTAL_BYTES < usize::MAX / 4,
                "the total cap must be finite"
            )
        };
    }

    #[cfg(test)]
    fn records_of(count: usize) -> crate::world::Records {
        (0..count)
            .map(|i| (format!("key-{i:04}"), serde_json::json!({"n": i})))
            .collect()
    }

    async fn put_document(store: &iroh_blobs::api::Store, name: &str, bytes: Vec<u8>) {
        let outcome = store.blobs().add_bytes(bytes).await.unwrap();
        store.tags().set(name, outcome.hash).await.unwrap();
    }

    #[tokio::test]
    async fn document_reader_limits_names_sizes_and_binary_content() {
        let store = test_store();
        put_document(&store, "notes.md", b"# hello\ntext".to_vec()).await;
        put_document(&store, "image.txt", vec![0xff, 0x00]).await;
        put_document(&store, "large.txt", vec![b'x'; MAX_DOCUMENT_BYTES + 1]).await;
        put_document(&store, ".private.md", b"hidden".to_vec()).await;
        put_document(&store, "evil\nworld.md", b"terminal injection".to_vec()).await;
        put_document(&store, "résumé.md", b"unicode ok".to_vec()).await;
        assert!(
            read_public_document(&store, "notes.md")
                .await
                .unwrap()
                .contains("# hello")
        );
        assert_eq!(
            read_public_document(&store, "missing.md").await,
            Err(DocumentError::Missing)
        );
        assert_eq!(
            read_public_document(&store, "image.txt").await,
            Err(DocumentError::Binary)
        );
        assert_eq!(
            read_public_document(&store, "large.txt").await,
            Err(DocumentError::TooLarge)
        );
        assert_eq!(
            read_public_document(&store, ".private.md").await,
            Err(DocumentError::InvalidName)
        );
        assert_eq!(
            read_public_document(&store, "../notes.md").await,
            Err(DocumentError::InvalidName)
        );
        assert_eq!(
            read_public_document(&store, "evil\nworld.md").await,
            Err(DocumentError::InvalidName)
        );
        assert!(
            read_public_document(&store, "résumé.md")
                .await
                .unwrap()
                .contains("unicode ok")
        );
        let listing = list_public_documents(&store, Some("notes")).await.unwrap();
        assert!(listing.contains("notes.md"));
        assert!(!listing.contains(".private.md"));
        assert!(!listing.contains("image.txt"));
        let all = list_public_documents(&store, None).await.unwrap();
        assert!(all.contains("résumé.md"));
        assert!(!all.contains("evil\nworld.md"));
    }

    #[tokio::test]
    async fn a_shell_lists_and_views_read_only_documents_from_its_server_store() {
        let store = test_store();
        put_document(
            &store,
            "guide.md",
            b"# SSH guide\nhello\x1b[2Jworld".to_vec(),
        )
        .await;
        put_document(&store, ".internal.md", b"hidden".to_vec()).await;
        let hub = lobby_hub();
        let invite = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .invite(Some("admin"), "ssh-docs".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start_with_store(hub, store).await;
        let session = connect_as(port, "cmd:", &capability).await;
        let mut channel = shell(&session).await;
        let mut terminal = vt100::Parser::new(24, 80, 0);
        read_until(&mut channel, &mut terminal, "world>").await;
        channel.data(&b"/documents guide\r"[..]).await.unwrap();
        read_until(&mut channel, &mut terminal, "guide.md").await;
        assert!(!terminal.screen().contents().contains(".internal.md"));
        channel.data(&b"/document guide.md\r"[..]).await.unwrap();
        read_until(&mut channel, &mut terminal, "# SSH guide").await;
        assert!(!terminal.screen().contents().contains("\x1b[2J"));
        server.abort();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_shell_plays_the_world_with_incremental_ansi_views() {
        use iroh_blobs::store::mem::MemStore;

        let blobs: iroh_blobs::api::Store = MemStore::new().into();
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, Some("admin".to_owned())).with_blob_store(blobs);
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm").to_vec();
        let hash = crate::world_session::module_hash(&wasm);
        service
            .install_wasm(Some("admin"), wasm, 99, &hash)
            .await
            .unwrap();
        let hub = WorldHub::single(service);
        let invite = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .invite(Some("admin"), "ssh".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(&invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start(hub).await;

        let session = connect(port, &capability).await;
        let mut channel = shell(&session).await;
        let mut terminal = vt100::Parser::new(24, 80, 0);
        let initial = read_until_bytes(&mut channel, &mut terminal, "count=0").await;
        assert!(initial.windows(3).any(|window| window == b"[2J"));
        channel.data(&b"inc"[..]).await.unwrap();
        let update = read_until_bytes(&mut channel, &mut terminal, "count=1").await;
        assert!(
            !update.windows(3).any(|window| window == b"[2J"),
            "subsequent screen update should not clear and repaint the terminal"
        );
        assert!(
            update.len() < initial.len(),
            "sparse update was {} bytes, initial full frame was {} bytes",
            update.len(),
            initial.len()
        );
        server.abort();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_shell_plays_the_world_in_command_mode_using_input_command() {
        use iroh_blobs::store::mem::MemStore;

        let blobs: iroh_blobs::api::Store = MemStore::new().into();
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, Some("admin".to_owned())).with_blob_store(blobs);
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm").to_vec();
        let hash = crate::world_session::module_hash(&wasm);
        service
            .install_wasm(Some("admin"), wasm, 99, &hash)
            .await
            .unwrap();
        let hub = WorldHub::single(service);
        let invite = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .invite(Some("admin"), "ssh-cmd".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(&invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start(hub).await;

        let session = connect_as(port, "cmd:", &capability).await;
        let mut channel = shell(&session).await;
        let mut terminal = vt100::Parser::new(24, 80, 0);
        read_until(&mut channel, &mut terminal, "count=0").await;
        channel.data(&b"/input 696e63\r"[..]).await.unwrap();
        read_until(&mut channel, &mut terminal, "count=1").await;
        // `/stats` sends `info` from the joined session; the host answers it
        // (it used to refuse it as "already joined").
        channel.data(&b"/stats\r"[..]).await.unwrap();
        read_until(&mut channel, &mut terminal, "current_sequence").await;
        assert!(
            !terminal.screen().contents().contains("already joined"),
            "{}",
            terminal.screen().contents()
        );
        server.abort();
    }
}
