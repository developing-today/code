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
    keys::{Algorithm, PrivateKey, ssh_key::HashAlg, ssh_key::LineEnding},
    server::{
        Auth, ChannelOpenHandle, Config, Handle, Handler, Msg, Server, Session, VerifiedPublicKey,
    },
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
use crate::world_terminal::{PROBE_QUERIES, PROBE_TIMEOUT, ProbeDemux};

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
/// Largest line the terminal presenter will accept for one form response.
const MAX_FORM_RESPONSE_BYTES: usize = crate::world_caps::MAX_EVENT_BYTES;
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
///
/// First-run creation races are expected (a service and a CLI starting
/// together): a reader that observes the winner's file mid-write retries
/// the parse for a bounded window instead of failing on a partial PEM,
/// and a creator that loses the `create_new` race reads the winner's key
/// instead of propagating `AlreadyExists`. A file that still does not
/// parse after the deadline is corrupt — it errors and is never
/// regenerated, so a valid key is never replaced on a parse error.
pub async fn load_or_create_host_key(path: &str) -> anyhow::Result<PrivateKey> {
    match tokio::fs::read_to_string(path).await {
        Ok(pem) => parse_host_key_after_creation_race(path, &pem).await,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
            let pem = key.to_openssh(LineEnding::LF)?;
            match crate::store::write_private_file(path, pem.as_bytes()).await {
                Ok(()) => Ok(key),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists) =>
                {
                    // Another first start won the creation race. Keep its
                    // key; never replace a key created after our first read.
                    let pem = tokio::fs::read_to_string(path).await?;
                    parse_host_key_after_creation_race(path, &pem).await
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// Parse a host-key PEM, retrying briefly while a concurrent first start
/// may still be writing the file. `first_pem` is the contents already
/// read; retries re-read the file. Mirrors the 1-second creation-race
/// window of `store::read_key_after_creation_race`, but parse-based: a
/// PEM has no fixed length to wait for. After the deadline the parse
/// error is returned as-is — the file is corrupt, not racing.
async fn parse_host_key_after_creation_race(
    path: &str,
    first_pem: &str,
) -> anyhow::Result<PrivateKey> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    let mut pem = first_pem.to_owned();
    loop {
        match PrivateKey::from_openssh(pem.as_str()) {
            Ok(key) => return Ok(key),
            Err(error) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(error.into());
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                pem = tokio::fs::read_to_string(path).await?;
            }
        }
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
        public_key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, russh::Error> {
        let Some(world) = user.strip_prefix(EXPLORE_PREFIX) else {
            return Ok(Auth::reject());
        };
        let Some(ed25519) = public_key.public_key.key_data().ed25519() else {
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
        // A raw shell behind a PTY owns a terminal to probe. Command mode is
        // line oriented, and an exec channel or PTY-less client gets no probe:
        // its bytes must keep the existing JSON-lines behavior.
        let probe_tty = first_resize.is_some() && !login.command_mode;
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
            records_after: None,
            errored: false,
            previous_screen: None,
            probe_tty,
            probe: None,
            active_form: None,
            form_line: Vec::new(),
            form_backlog: VecDeque::new(),
            form_after_cr: false,
            form_echo_pending: Vec::new(),
            discarding_form_line: false,
            closed_form: None,
            retry_form_token: None,
            pending_form_answer: None,
        };
        let hub = self.hub.clone();
        tokio::spawn(async move {
            io.enter_alt_screen().await;
            io.start_terminal_probe().await;
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
    /// Cursor that produced the in-flight records page (`None` before the
    /// first page, whose request sends `after: null`); the page's
    /// `next_after` must strictly advance past it (#6615).
    records_after: Option<String>,
    errored: bool,
    /// Last screen successfully written to this shell's terminal. A missing
    /// value forces the next screen view to repaint the whole terminal.
    previous_screen: Option<crate::world_presentation::ScreenSurface>,
    /// Whether this session owns an interactive PTY, so a startup terminal
    /// probe is allowed. False for exec channels and PTY-less callers.
    probe_tty: bool,
    /// Bounded startup probe state while the reply window is open.
    probe: Option<SshProbe>,
    /// Form view the terminal is currently collecting a response for.
    active_form: Option<crate::world_presentation::FormSurface>,
    /// Bounded line being entered for the active form.
    form_line: Vec<u8>,
    /// Bytes following the current form line in already-received SSH packets.
    form_backlog: VecDeque<u8>,
    /// Whether a CR terminator was consumed, so a following LF is swallowed.
    form_after_cr: bool,
    /// At most the incomplete UTF-8 scalar currently waiting to be echoed.
    form_echo_pending: Vec<u8>,
    /// Ignore the remainder of an invalid or oversized answer through its terminator.
    discarding_form_line: bool,
    /// Last closed form presentation and all token aliases; stale redraws must not reactivate it.
    closed_form: Option<(String, Vec<String>)>,
    /// A retry token issued by the host, paired with the token on the stale view.
    retry_form_token: Option<(String, String, String)>,
    /// Encoded answer retained across cancellation of `recv` while echoing input.
    pending_form_answer: Option<String>,
}

/// The one startup probe window of a raw SSH session.
#[derive(Debug)]
struct SshProbe {
    demux: ProbeDemux,
    deadline: tokio::time::Instant,
}

enum FormInputProgress {
    Pending {
        echo: Vec<u8>,
    },
    Complete {
        line: Vec<u8>,
        echo: Vec<u8>,
    },
    Invalid {
        message: &'static str,
        echo: Vec<u8>,
    },
}

/// Echo a printable ASCII byte immediately or complete only the pending UTF-8
/// scalar. The accumulated answer is validated once, when its line is submitted.
fn echo_form_byte(byte: u8, pending: &mut Vec<u8>) -> Vec<u8> {
    if pending.is_empty() {
        if byte.is_ascii() {
            return if byte.is_ascii_graphic() || byte == b' ' {
                vec![byte]
            } else {
                Vec::new()
            };
        }
        if matches!(byte, 0xc2..=0xdf | 0xe0..=0xef | 0xf0..=0xf4) {
            pending.push(byte);
        }
        return Vec::new();
    }

    if byte & 0xc0 != 0x80 {
        pending.clear();
        return echo_form_byte(byte, pending);
    }
    pending.push(byte);
    let expected = match pending[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => {
            pending.clear();
            return Vec::new();
        }
    };
    if pending.len() < expected {
        return Vec::new();
    }
    let echo = std::str::from_utf8(pending)
        .map(terminal_text)
        .unwrap_or_default()
        .into_bytes();
    pending.clear();
    echo
}

/// Discard an invalid/overlong input suffix through one line terminator,
/// preserving later bytes and CRLF state for the next read.
fn discard_form_input_line(backlog: &mut VecDeque<u8>, after_cr: &mut bool) -> bool {
    while let Some(byte) = backlog.pop_front() {
        let swallowed_lf = std::mem::take(after_cr);
        if byte == b'\n' && swallowed_lf {
            continue;
        }
        match byte {
            b'\r' => {
                *after_cr = true;
                return true;
            }
            b'\n' => return true,
            _ => {}
        }
    }
    false
}

/// Consume queued form bytes through at most one line terminator. Remaining
/// packet bytes stay in the queue for the next form answer or world input.
fn consume_form_input(
    backlog: &mut VecDeque<u8>,
    line: &mut Vec<u8>,
    after_cr: &mut bool,
    echo_pending: &mut Vec<u8>,
) -> FormInputProgress {
    let mut echo = Vec::new();
    while let Some(byte) = backlog.pop_front() {
        let swallowed_lf = std::mem::take(after_cr);
        if byte == b'\n' && swallowed_lf {
            continue;
        }
        match byte {
            b'\r' | b'\n' => {
                *after_cr = byte == b'\r';
                echo_pending.clear();
                echo.extend_from_slice(b"\r\n");
                return FormInputProgress::Complete {
                    line: std::mem::take(line),
                    echo,
                };
            }
            0x08 | 0x7f => {
                echo_pending.clear();
                if std::str::from_utf8(line).is_err() {
                    while !line.is_empty() && std::str::from_utf8(line).is_err() {
                        line.pop();
                    }
                } else if let Some(last) = std::str::from_utf8(line)
                    .ok()
                    .and_then(|line| line.char_indices().next_back().map(|(index, _)| index))
                {
                    line.truncate(last);
                    echo.extend_from_slice(b"\x08 \x08");
                }
            }
            byte if byte.is_ascii_control() => {
                line.clear();
                echo_pending.clear();
                return FormInputProgress::Invalid {
                    message: "form input cannot contain control characters",
                    echo,
                };
            }
            byte if line.len() < MAX_FORM_RESPONSE_BYTES => {
                line.push(byte);
                echo.extend_from_slice(&echo_form_byte(byte, echo_pending));
            }
            _ => {
                line.clear();
                echo_pending.clear();
                return FormInputProgress::Invalid {
                    message: "form response line is too large",
                    echo,
                };
            }
        }
    }
    FormInputProgress::Pending { echo }
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

mod form_io;
use form_io::{
    FormBytes, discard_form_lf_after_cr, form_closed_aliases, form_retry_alias,
    recv_bytes_through_probe, take_pending_form_answer,
};

impl SshIo {
    /// Remember every token that may still appear in a guest's stale view.
    fn remember_closed_form(&mut self) {
        let Some((form_id, current_token)) = self.active_form.as_ref().and_then(|form| {
            form.presentation_token
                .as_ref()
                .map(|token| (form.form_id.clone(), token.clone()))
        }) else {
            return;
        };
        let aliases = form_closed_aliases(&form_id, &current_token, self.retry_form_token.as_ref());
        self.closed_form = Some((form_id, aliases));
    }

    /// Discard an invalid form-line suffix without turning it into a new answer.
    async fn discard_invalid_form_suffix(&mut self) -> Option<Inbound> {
        loop {
            if discard_form_input_line(&mut self.form_backlog, &mut self.form_after_cr) {
                self.discarding_form_line = false;
                return None;
            }
            match recv_bytes_through_probe(&mut self.frames, &mut self.probe).await {
                FormBytes::Bytes(bytes) => self.form_backlog.extend(bytes),
                FormBytes::Resize(resize) => {
                    self.previous_screen = None;
                    return Some(Inbound::Text(raw_frame(&resize)));
                }
                FormBytes::Closed => return Some(Inbound::Closed),
            }
        }
    }

    /// Swallow a CR's optional LF before routing post-form bytes as world input.
    async fn clear_pending_form_lf(&mut self) -> Option<Inbound> {
        loop {
            if discard_form_lf_after_cr(&mut self.form_backlog, &mut self.form_after_cr) {
                return None;
            }
            match recv_bytes_through_probe(&mut self.frames, &mut self.probe).await {
                FormBytes::Bytes(bytes) => self.form_backlog.extend(bytes),
                FormBytes::Resize(resize) => {
                    self.previous_screen = None;
                    return Some(Inbound::Text(raw_frame(&resize)));
                }
                FormBytes::Closed => return Some(Inbound::Closed),
            }
        }
    }

    /// Encode one complete form answer; malformed JSON stays local to the SSH prompt.
    fn form_answer_frame(&self, line: &str) -> Result<String, String> {
        let form = self
            .active_form
            .as_ref()
            .ok_or_else(|| "no form is active".to_owned())?;
        terminal_form_answer_frame(form, line)
    }

    /// Collect one bounded UTF-8 answer, echo edits, and retain packet bytes after its newline.
    async fn recv_form_answer(&mut self) -> Inbound {
        loop {
            if self.discarding_form_line {
                if let Some(inbound) = self.discard_invalid_form_suffix().await {
                    return inbound;
                }
                continue;
            }
            if self.form_backlog.is_empty() {
                match recv_bytes_through_probe(&mut self.frames, &mut self.probe).await {
                    FormBytes::Bytes(bytes) => self.form_backlog.extend(bytes),
                    FormBytes::Resize(resize) => {
                        self.previous_screen = None;
                        return Inbound::Text(raw_frame(&resize));
                    }
                    FormBytes::Closed => return Inbound::Closed,
                }
            }
            let progress = consume_form_input(
                &mut self.form_backlog,
                &mut self.form_line,
                &mut self.form_after_cr,
                &mut self.form_echo_pending,
            );
            let (answer, invalid, echo) = match progress {
                FormInputProgress::Pending { echo } => (None, None, echo),
                FormInputProgress::Complete { line, echo } => (Some(line), None, echo),
                FormInputProgress::Invalid { message, echo } => (None, Some(message), echo),
            };
            if invalid.is_some() {
                self.discarding_form_line = true;
            }
            let mut answer_error = None;
            if let Some(answer) = answer {
                match String::from_utf8(answer)
                    .map_err(|_| "form response must be UTF-8".to_owned())
                    .and_then(|line| self.form_answer_frame(&line))
                {
                    Ok(frame) => self.pending_form_answer = Some(frame),
                    Err(message) => answer_error = Some(message),
                }
            }
            if !echo.is_empty() && self.handle.data(self.channel, echo).await.is_err() {
                return Inbound::Closed;
            }
            if let Some(frame) = self.pending_form_answer.take() {
                return Inbound::Text(frame);
            }
            if let Some(message) = invalid {
                let prompt = self
                    .active_form
                    .as_ref()
                    .map_or_else(String::new, terminal_form_prompt);
                if self
                    .handle
                    .data(
                        self.channel,
                        terminal_text(&format!("\r\n{message}\r\n{prompt}")),
                    )
                    .await
                    .is_err()
                {
                    return Inbound::Closed;
                }
                continue;
            }
            if let Some(message) = answer_error {
                let prompt = self
                    .active_form
                    .as_ref()
                    .map_or_else(String::new, terminal_form_prompt);
                if self
                    .handle
                    .data(
                        self.channel,
                        terminal_text(&format!("\r\n{message}\r\n{prompt}")),
                    )
                    .await
                    .is_err()
                {
                    return Inbound::Closed;
                }
            }
        }
    }

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

    /// Send the one startup probe burst, never retried per feature (F13.4).
    ///
    /// Only a raw shell behind a PTY probes. A failed write leaves the
    /// session on the existing behavior with no probe window open, exactly
    /// like a caller that never had a terminal.
    async fn start_terminal_probe(&mut self) {
        if !self.probe_tty || self.probe.is_some() {
            return;
        }
        if self.handle.data(self.channel, PROBE_QUERIES).await.is_err() {
            return;
        }
        self.probe = Some(SshProbe {
            demux: ProbeDemux::new(),
            deadline: tokio::time::Instant::now() + PROBE_TIMEOUT,
        });
    }

    /// Close the probe window, returning any held bytes for replay.
    ///
    /// The selected tier is logged only: this slice enables the Tier 3 safe
    /// fallback and leaves every richer presenter unreachable.
    fn close_terminal_probe(&mut self) -> Vec<u8> {
        let Some(mut probe) = self.probe.take() else {
            return Vec::new();
        };
        let tier = probe.demux.tier();
        tracing::debug!(
            ?tier,
            "terminal capability probe closed; Tier 3 fallback stays active"
        );
        probe.demux.expire()
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
        if let Some(inbound) = take_pending_form_answer(&mut self.pending_form_answer) {
            return inbound;
        }
        if self.discarding_form_line
            && let Some(inbound) = self.discard_invalid_form_suffix().await
        {
            return inbound;
        }
        if self.active_form.is_none()
            && let Some(inbound) = self.clear_pending_form_lf().await
        {
            return inbound;
        }
        if self.active_form.is_none() && !self.form_backlog.is_empty() {
            let bytes: Vec<_> = self.form_backlog.drain(..).collect();
            match self.mode {
                ShellMode::Raw => return Inbound::Text(raw_frame(&SshInputFrame::Bytes(bytes))),
                ShellMode::Command => self.commands.extend(self.lines.push(&bytes)),
            }
        }
        if self.active_form.is_some() {
            return self.recv_form_answer().await;
        }
        match self.mode {
            ShellMode::Raw => loop {
                let Some(deadline) = self.probe.as_ref().map(|probe| probe.deadline) else {
                    return self
                        .frames
                        .recv()
                        .await
                        .map_or(Inbound::Closed, |input| Inbound::Text(raw_frame(&input)));
                };
                // Cancel-safe: the probe stays in place across the await, and
                // bytes received in an earlier call are held in its demux.
                let received = tokio::select! {
                    frame = self.frames.recv() => Some(frame),
                    () = tokio::time::sleep_until(deadline) => None,
                };
                match received {
                    Some(Some(SshInputFrame::Bytes(bytes))) => {
                        let input = self
                            .probe
                            .as_mut()
                            .map_or_else(Vec::new, |probe| probe.demux.push(&bytes));
                        if !input.is_empty() {
                            return Inbound::Text(raw_frame(&SshInputFrame::Bytes(input)));
                        }
                    }
                    Some(Some(resize @ SshInputFrame::Resize(..))) => {
                        self.previous_screen = None;
                        return Inbound::Text(raw_frame(&resize));
                    }
                    Some(None) => return Inbound::Closed,
                    None => {
                        let held = self.close_terminal_probe();
                        if !held.is_empty() {
                            return Inbound::Text(raw_frame(&SshInputFrame::Bytes(held)));
                        }
                    }
                }
            },
            ShellMode::Command => loop {
                if let Some(command) = self.commands.pop_front() {
                    if let Some(frame) = command_frame(&command, &self.capability) {
                        if let WorldCommand::Records { prefix } = &command {
                            self.records_prefix = Some(prefix.clone());
                            self.records_drained = 0;
                            self.records_after = None;
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
        if value["type"] == "form_result_status" {
            let form_id = value["form_id"].as_str().unwrap_or_default();
            if self
                .active_form
                .as_ref()
                .is_some_and(|form| form.form_id == form_id)
            {
                if value["ok"].as_bool() == Some(true) {
                    self.remember_closed_form();
                    self.active_form = None;
                    self.form_line.clear();
                    self.retry_form_token = None;
                    let message = if matches!(self.mode, ShellMode::Command) {
                        "\r\nForm submitted.\r\nworld> ".to_owned()
                    } else {
                        "\r\nForm submitted.\r\n".to_owned()
                    };
                    return self.handle.data(self.channel, message).await.is_ok();
                }
                if let Some(token) = value["presentation_token"].as_str()
                    && let Some(form) = &mut self.active_form
                {
                    if let Some(previous_token) = form.presentation_token.clone() {
                        self.retry_form_token = Some(form_retry_alias(
                            self.retry_form_token.as_ref(),
                            &form.form_id,
                            &previous_token,
                            token,
                        ));
                    }
                    form.presentation_token = Some(token.to_owned());
                    self.form_line.clear();
                    let message = terminal_text(&format!(
                        "\r\nThat answer was not accepted; review the fields and try again.\r\n{}",
                        terminal_form_prompt(form)
                    ));
                    return self.handle.data(self.channel, message).await.is_ok();
                }
                let error = value["error"].as_str().unwrap_or("form_unavailable");
                let reason = value["reason"].as_str().unwrap_or("unknown");
                self.remember_closed_form();
                self.active_form = None;
                self.form_line.clear();
                self.retry_form_token = None;
                let message = terminal_text(&format!(
                    "\r\nForm closed ({error}: {reason}). Returning to world input.\r\n"
                ));
                return self.handle.data(self.channel, message).await.is_ok();
            }
            let message = terminal_text("\r\nForm response is no longer active.\r\n");
            return self.handle.data(self.channel, message).await.is_ok();
        }
        if value["type"] == "view"
            && let Some(surface) = value.get("surface")
            && let Ok(crate::world_presentation::WorldSurface::Form(mut form)) =
                serde_json::from_value(surface.clone())
        {
            self.previous_screen = None;
            if let Some((closed_id, closed_tokens)) = &self.closed_form
                && closed_id == &form.form_id
                && form
                    .presentation_token
                    .as_ref()
                    .is_some_and(|token| closed_tokens.contains(token))
            {
                return true;
            }
            if let Some((retry_id, old_token, new_token)) = &self.retry_form_token
                && retry_id == &form.form_id
                && form.presentation_token.as_ref() == Some(old_token)
            {
                // A redraw may still carry the program's old token after a
                // rejected answer. Keep the host-issued retry token active.
                form.presentation_token = Some(new_token.clone());
            } else if self
                .retry_form_token
                .as_ref()
                .is_some_and(|(id, _, _)| id == &form.form_id)
            {
                self.retry_form_token = None;
            }
            let input = terminal_form_input_manifest();
            let message = if form.presentation_token.is_some()
                && form.validate_device_input(&input).is_ok()
            {
                let prompt = terminal_form_prompt(&form);
                let changed = self.active_form.as_ref().is_none_or(|active| {
                    active.form_id != form.form_id
                        || active.presentation_token != form.presentation_token
                });
                if changed {
                    self.form_line.clear();
                }
                self.closed_form = None;
                self.active_form = Some(form);
                format!("\x1b[H\x1b[2J{}", terminal_text(&prompt))
            } else {
                self.active_form = None;
                self.form_line.clear();
                self.retry_form_token = None;
                format!(
                    "\r\nThis terminal cannot safely present the current form.\r\n{}",
                    terminal_text(&form.text_projection())
                )
            };
            return self.handle.data(self.channel, message).await.is_ok();
        }
        if value["type"] == "program_changed" || value["type"] == "view" {
            self.active_form = None;
            self.form_line.clear();
            self.closed_form = None;
            self.retry_form_token = None;
        }
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
                self.records_after = None;
                let message = value["message"].as_str().unwrap_or("error");
                self.last_error = Some(format!("\r\n{}\r\n", terminal_text(message)));
                self.last_error.clone().unwrap_or_default()
            }
            Some("records") => {
                let records = value["records"].as_object();
                let sequence = value["sequence"].as_u64().unwrap_or_default();
                // Validate the cursor before trusting it (#6615): a
                // present-but-non-string `next_after` is a malformed page,
                // not a clean end-of-pages, and a cursor that does not
                // strictly advance past the one that produced this page
                // would re-request the same page until the byte cap stops
                // the drain (~18,700 re-requests in the filing's probe).
                // The advance rule is also why no page-count bound is
                // needed beside the byte bound below: every continuation
                // moves the cursor forward through the host's key space,
                // so pages cannot repeat, and the rendered-byte total
                // still caps the drain's size. Oracle: the `world_net`
                // records cursor contract (#6505/#6578).
                let produced_by = self.records_after.clone().unwrap_or_default();
                let (after, notice) = match records_cursor(value.get("next_after"), &produced_by) {
                    RecordsCursor::Advance(next) => (Some(next), None),
                    RecordsCursor::End => (None, None),
                    RecordsCursor::Malformed => (
                        None,
                        Some("[records truncated: host sent a malformed next-page cursor]\n"),
                    ),
                    RecordsCursor::Stalled => (
                        None,
                        Some("[records truncated: host cursor did not advance]\n"),
                    ),
                };
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
                    self.records_after = Some(after.clone());
                    self.commands
                        .push_back(WorldCommand::RecordsContinue { prefix, after });
                } else {
                    self.records_prefix = None;
                    self.records_after = None;
                }
                if truncated {
                    format!(
                        "\r\n{}\r\n{}",
                        terminal_text(&text),
                        terminal_text("[records truncated: too much output]\n")
                    )
                } else if let Some(notice) = notice {
                    format!("\r\n{}\r\n{}", terminal_text(&text), terminal_text(notice))
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
            anyhow::ensure!(
                digits.bytes().all(|byte| byte.is_ascii_digit()),
                "`{digits}` is not a port"
            );
            let port = digits
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("`{digits}` is not a port"))?;
            anyhow::ensure!(port != 0, "`{digits}` is not a port");
            Ok(port)
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
        // #3267: whitespace, control characters, and `/` in the user or
        // host are never legitimate — without this check they parse and
        // fail later inside russh as opaque DNS/auth errors, and a
        // newline-bearing host lands verbatim in error messages and the
        // known-hosts lookup key. The bracketed-IPv6 host is held to the
        // same rule: valid IPv6 literals contain none of these.
        let has_bad_chars = |part: &str| {
            part.chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '/')
        };
        anyhow::ensure!(
            !has_bad_chars(user) && !has_bad_chars(host),
            "`{text}` is not [user@]host[:port]: user and host must not \
             contain whitespace, control characters, or `/`"
        );
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

/// What a records page's `next_after` means for the drain, judged
/// against `produced_by` — the cursor that produced this page (the
/// empty string before the first page, whose request sends
/// `after: null`). Factored out of the `records` frame arm so the
/// contract can be unit-tested without a live session (#6615).
#[derive(Debug, PartialEq, Eq)]
enum RecordsCursor {
    /// No cursor (absent or null): the listing is complete.
    End,
    /// A cursor that strictly advances: fetch the next page after it.
    Advance(String),
    /// A present, non-string cursor: a malformed page, never a clean
    /// end-of-pages.
    Malformed,
    /// A cursor that does not advance past `produced_by` (a repeat,
    /// a step backwards, or the empty cursor at the start): following
    /// it would re-request the page just rendered.
    Stalled,
}

fn records_cursor(next_after: Option<&serde_json::Value>, produced_by: &str) -> RecordsCursor {
    match next_after {
        None | Some(serde_json::Value::Null) => RecordsCursor::End,
        Some(serde_json::Value::String(cursor)) if cursor.as_str() > produced_by => {
            RecordsCursor::Advance(cursor.clone())
        }
        Some(serde_json::Value::String(_)) => RecordsCursor::Stalled,
        Some(_) => RecordsCursor::Malformed,
    }
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
/// one participant cannot drive another participant's terminal. Unicode
/// format characters (Cf, per the shared `crate::is_format_char` set) are
/// dropped for the same reason: a bidi override or zero-width character in
/// one participant's text could otherwise reorder or falsify the terminal
/// output around it (design #3455). The accepted cost is that `ZWJ`-joined
/// emoji render as separate glyphs here; the stored text is untouched.
fn terminal_text(data: &str) -> String {
    data.chars()
        .filter(|ch| (!ch.is_control() || matches!(ch, '\n' | '\t')) && !crate::is_format_char(*ch))
        .collect::<String>()
        .replace('\n', "\r\n")
}

/// Local input affordances of the SSH terminal presenter. This is deliberately
/// distinct from the headless server's `device.info` manifest.
fn terminal_form_input_manifest() -> serde_json::Value {
    json!({
        "keys": 128,
        "encoder": false,
        "pointer": false,
        "text_entry": "keyboard",
    })
}

/// Safe, line-oriented prompt for the SSH presenter. The user's response is
/// parsed as JSON data and sent in a typed session frame; it is never shell or
/// markup input.
fn terminal_form_prompt(form: &crate::world_presentation::FormSurface) -> String {
    use crate::world_presentation::FormActionKind;
    use crate::world_presentation::FormField;

    let actions = form
        .actions
        .iter()
        .map(|action| {
            let kind = match action.kind {
                FormActionKind::Submit => "submit",
                FormActionKind::Cancel => "cancel",
            };
            format!("{} — {} ({kind})", action.id, action.label)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let fields = form
        .fields
        .iter()
        .map(|field| {
            let (id, label, details) = match field {
                FormField::Text {
                    id,
                    label,
                    required,
                    max_length,
                } => (
                    id,
                    label,
                    format!("text; required={required}; max_length={max_length}"),
                ),
                FormField::Choice {
                    id,
                    label,
                    required,
                    options,
                } => {
                    let options = options
                        .iter()
                        .map(|option| terminal_json_string(option))
                        .collect::<Vec<_>>()
                        .join(", ");
                    (
                        id,
                        label,
                        format!("choice; required={required}; options=[{options}]"),
                    )
                }
                FormField::Number {
                    id,
                    label,
                    required,
                    min,
                    max,
                } => (
                    id,
                    label,
                    format!("number; required={required}; min={min}; max={max}"),
                ),
                FormField::Date {
                    id,
                    label,
                    required,
                    min,
                    max,
                } => (
                    id,
                    label,
                    format!("date; required={required}; min={min:?}; max={max:?}"),
                ),
                FormField::Confirm {
                    id,
                    label,
                    required,
                } => (id, label, format!("confirm; required={required}")),
            };
            format!(
                "{} — {} [{}]",
                terminal_text(label),
                terminal_text(id),
                terminal_text(&details)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{}\nFields (values are keyed by field ID):\n{}\nActions:\n{}\nEnter {{\"action\":\"<id>\",\"values\":{{...}}}} or /cancel.\nform> ",
        terminal_text(&form.title),
        fields,
        actions
    )
}

/// JSON-quote an option while escaping terminal-invisible format characters,
/// so the displayed value remains copyable and compares exactly on submit.
fn terminal_json_string(value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned());
    let mut escaped = String::with_capacity(quoted.len());
    for ch in quoted.chars() {
        if crate::is_format_char(ch) {
            let mut units = [0; 2];
            for unit in ch.encode_utf16(&mut units).iter() {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{unit:04x}");
            }
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

/// Validate one complete JSON response line and encode it as a typed session frame.
fn terminal_form_answer_frame(
    form: &crate::world_presentation::FormSurface,
    line: &str,
) -> Result<String, String> {
    let (action, values) = if line.trim() == "/cancel" {
        let action = form
            .actions
            .iter()
            .find(|action| action.kind == crate::world_presentation::FormActionKind::Cancel)
            .ok_or_else(|| "this form has no cancel action".to_owned())?;
        (action.id.clone(), serde_json::Map::new())
    } else {
        let value: serde_json::Value = serde_json::from_str(line)
            .map_err(|_| "enter a JSON object with action and values".to_owned())?;
        let object = value
            .as_object()
            .ok_or_else(|| "form response must be a JSON object".to_owned())?;
        if object.len() != 2 || !object.contains_key("action") || !object.contains_key("values") {
            return Err("form response requires exactly action and values".to_owned());
        }
        let action = object["action"]
            .as_str()
            .ok_or_else(|| "form response action must be a string".to_owned())?;
        let values = object["values"]
            .as_object()
            .cloned()
            .ok_or_else(|| "form response values must be an object".to_owned())?;
        (action.to_owned(), values)
    };
    let token = form
        .presentation_token
        .as_deref()
        .ok_or_else(|| "form has no active presentation token".to_owned())?;
    let frame = serde_json::json!({
        "type": "form_result",
        "form_id": form.form_id,
        "presentation_token": token,
        "action": action,
        "values": values,
    })
    .to_string();
    if frame.len() > crate::frames::MAX_FRAME_BYTES {
        return Err("form response exceeds the frame size limit".to_owned());
    }
    Ok(frame)
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

    #[tokio::test]
    async fn test_concurrent_first_starts_converge_on_the_winners_host_key() {
        // All callers line up behind a barrier so their first reads race
        // the winner's create_new write: the losers must load the winner's
        // key (via the AlreadyExists winner-read and/or the parse retry),
        // never error and never end up with a different key.
        for _round in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ssh_host_key").display().to_string();
            let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(8));
            let mut handles = Vec::new();
            for _ in 0..8 {
                let barrier = std::sync::Arc::clone(&barrier);
                let path = path.clone();
                handles.push(tokio::spawn(async move {
                    barrier.wait().await;
                    load_or_create_host_key(&path).await
                }));
            }
            let mut fingerprints = Vec::new();
            for handle in handles {
                let key = handle.await.unwrap().unwrap();
                fingerprints.push(host_key_fingerprint(&key));
            }
            assert!(
                fingerprints.iter().all(|f| f == &fingerprints[0]),
                "concurrent first starts diverged: {fingerprints:?}"
            );
            let on_disk = load_or_create_host_key(&path).await.unwrap();
            assert_eq!(host_key_fingerprint(&on_disk), fingerprints[0]);
        }
    }

    #[tokio::test]
    async fn test_a_partially_written_host_key_pem_is_retried_until_it_parses() {
        // The interleaving from #3465: the reader observes the winner's
        // file between create_new and the completed write. The partial
        // PEM is already on disk before the load starts, so the first
        // read deterministically sees it; the load must retry until the
        // writer finishes, not fail with a parse error.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ssh_host_key");
        let winner = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let pem = winner.to_openssh(LineEnding::LF).unwrap();
        let pem_text = pem.as_str().to_owned();
        tokio::fs::write(&path, &pem_text[..pem_text.len() / 2])
            .await
            .unwrap();
        let completer_path = path.clone();
        let completer = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            tokio::fs::write(&completer_path, pem_text).await.unwrap();
        });
        let loaded = load_or_create_host_key(path.to_str().unwrap())
            .await
            .unwrap();
        completer.await.unwrap();
        assert_eq!(host_key_fingerprint(&loaded), host_key_fingerprint(&winner));
    }

    #[tokio::test]
    async fn test_an_unparseable_host_key_errors_and_is_never_regenerated() {
        // Past the creation-race deadline a bad PEM is corrupt, not
        // racing: the load errors and the file is left exactly as found.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ssh_host_key");
        let garbage =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nnot-a-key\n-----END OPENSSH PRIVATE KEY-----\n";
        tokio::fs::write(&path, garbage).await.unwrap();
        let result = load_or_create_host_key(path.to_str().unwrap()).await;
        assert!(result.is_err(), "corrupt host key must error, not load");
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, garbage, "corrupt host key must never be regenerated");
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
    async fn a_raw_shell_probes_once_and_keeps_probe_replies_out_of_world_input() {
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
            .invite(Some("admin"), "ssh-probe".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start(hub).await;
        let session = connect(port, &capability).await;
        let mut channel = shell(&session).await;

        // The burst is written once, behind one deadline.
        let queries = String::from_utf8_lossy(PROBE_QUERIES).into_owned();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while sequence_count(&bytes, &queries) == 0 {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => bytes.extend_from_slice(&data),
                    Some(_) => {}
                    None => break,
                }
            }
        })
        .await
        .expect("timed out waiting for the terminal probe");
        assert_eq!(
            sequence_count(&bytes, &queries),
            1,
            "probe burst must be written exactly once"
        );

        // Answer inside the window with user keys on both sides of replies.
        let reply = [
            b"j".as_slice(),
            b"\x1bP1+r436F=323536\x1b\\",
            b"k".as_slice(),
            b"\x1b[12;40R",
        ]
        .concat();
        channel.data(&reply[..]).await.unwrap();

        let mut typed = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while typed.len() < 2 {
                if let Ok(event) = events.recv().await
                    && let crate::world::WorldEventKind::Input(bytes) = event.kind
                {
                    typed.extend_from_slice(&bytes);
                }
            }
        })
        .await
        .expect("typed keys did not reach the world");
        assert_eq!(
            typed,
            b"jk".to_vec(),
            "only user keys belong in world input"
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_shell_without_a_pty_sends_no_probe_bytes() {
        let hub = lobby_hub();
        let invite = hub
            .lease(None, Some("admin"))
            .await
            .unwrap()
            .service()
            .invite(Some("admin"), "ssh-no-pty".to_owned(), None, None)
            .await
            .unwrap();
        let capability = serde_json::to_value(invite).unwrap()["capability"]
            .as_str()
            .unwrap()
            .to_owned();
        let (port, server) = start(hub).await;
        let session = connect(port, &capability).await;
        // No `request_pty`: the raw shell still works but owns no terminal.
        let mut channel = session.channel_open_session().await.unwrap();
        channel.request_shell(true).await.unwrap();

        let queries = String::from_utf8_lossy(PROBE_QUERIES).into_owned();
        let mut bytes = Vec::new();
        let _ = tokio::time::timeout(Duration::from_millis(600), async {
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => bytes.extend_from_slice(&data),
                    Some(_) => {}
                    None => break,
                }
            }
        })
        .await;
        assert_eq!(
            sequence_count(&bytes, &queries),
            0,
            "a TTY-less caller must receive no probe bytes"
        );
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
        assert_eq!(
            parse("me@[2001:db8::1]:2222").unwrap(),
            target("me", "2001:db8::1", 2222)
        );
        for bad in [
            "",
            "host:",
            "host:0",
            "[::1]:0",
            "host:+22",
            "host:-22",
            "host:٢٢",
            "host:99999",
            "::1",
            "[::1",
            "@host",
            "[::1]x",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        // #3267: whitespace, control characters, and `/` in the user or
        // the host (the bracketed-IPv6 host included) refuse at parse
        // time instead of failing later inside russh.
        for bad in [
            "exam ple.org",
            "ada@exam ple.org",
            " @host",
            "ada @host",
            "ada@host ",
            "ada@ho/st",
            "a/b@example.org",
            "[:: 1]",
            "ada@[:: 1]",
            "ada@example.org\n.evil",
            "us\ter@host",
            "ada@ho\rst",
        ] {
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
        sent.push(check(
            &serde_json::json!({
                "type": "form_result",
                "form_id": "form-1",
                "presentation_token": "0123456789abcdef0123456789abcdef",
                "action": "submit",
                "values": {"name": "Ada"}
            })
            .to_string(),
            Phase::Joined,
        ));
        assert_eq!(
            sent,
            [
                "chat",
                "input",
                "records",
                "records",
                "info",
                "caps",
                "records",
                "form_result"
            ]
        );
        let status = serde_json::json!({
            "type": "form_result_status",
            "form_id": "form-1",
            "ok": false,
            "presentation_token": "fedcba9876543210fedcba9876543210",
            "error": "bad_args",
            "reason": "field_invalid",
        });
        assert_eq!(
            contract.check_server_frame(&status).unwrap(),
            "form_result_status"
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
    fn terminal_text_drops_format_characters_but_keeps_layout() {
        // Bidi overrides and zero-width characters never reach the terminal,
        // even without any control character present (#3455 Phase 3b).
        assert_eq!(terminal_text("a\u{202e}b"), "ab");
        assert_eq!(terminal_text("a\u{200b}b"), "ab");
        assert_eq!(terminal_text("a\u{feff}b"), "ab");
        // The shared Cf set is the gate, not a bidi-only subset.
        assert!(!terminal_text("\u{00ad}").chars().any(|c| c == '\u{00ad}'));
        // Layout survivors are unchanged: LF/TAB are kept (as CRLF/TAB).
        assert_eq!(terminal_text("a\nb\tc"), "a\r\nb\tc");
        // Ordinary text, including non-joiner emoji bases, is byte-identical.
        assert_eq!(terminal_text("naïve-✦サーバー"), "naïve-✦サーバー");
    }

    #[test]
    fn test_form_retry_alias_preserves_the_original_view_token_across_retries() {
        let first = form_retry_alias(None, "form-1", "T0", "T1");
        assert_eq!(
            first,
            ("form-1".to_owned(), "T0".to_owned(), "T1".to_owned())
        );
        // The guest still renders T0; a second rejection advances the
        // current token but must keep T0 as the view alias.
        let second = form_retry_alias(Some(&first), "form-1", "T1", "T2");
        assert_eq!(
            second,
            ("form-1".to_owned(), "T0".to_owned(), "T2".to_owned())
        );
        // A different form, or a rejected token that is not the current
        // retry token, starts a fresh alias from the rejected token.
        let other = form_retry_alias(Some(&second), "form-2", "T9", "T10");
        assert_eq!(
            other,
            ("form-2".to_owned(), "T9".to_owned(), "T10".to_owned())
        );
        let stale = form_retry_alias(Some(&second), "form-1", "T7", "T8");
        assert_eq!(
            stale,
            ("form-1".to_owned(), "T7".to_owned(), "T8".to_owned())
        );
    }

    #[test]
    fn test_closed_retried_form_suppresses_original_view_token() {
        let first = form_retry_alias(None, "form-1", "T0", "T1");
        let retry = form_retry_alias(Some(&first), "form-1", "T1", "T2");
        let closed = form_closed_aliases("form-1", "T2", Some(&retry));

        assert!(closed.contains(&"T0".to_owned()));
        assert!(closed.contains(&"T2".to_owned()));
        assert!(!form_closed_aliases("form-2", "T2", Some(&retry)).contains(&"T0".to_owned()));
    }

    #[test]
    fn test_form_answer_frame_survives_recv_cancellation() {
        let mut pending = Some("encoded-form-result".to_owned());

        let inbound = take_pending_form_answer(&mut pending);

        assert!(
            matches!(inbound, Some(Inbound::Text(ref frame)) if frame == "encoded-form-result")
        );
        assert!(pending.is_none());
    }

    #[test]
    fn test_form_crlf_suppression_handles_split_packet_and_preserves_next_input() {
        let mut backlog = VecDeque::new();
        let mut after_cr = true;
        assert!(!discard_form_lf_after_cr(&mut backlog, &mut after_cr));
        assert!(after_cr);

        backlog.extend([b'\n', b'/']);
        assert!(discard_form_lf_after_cr(&mut backlog, &mut after_cr));
        assert!(!after_cr);
        assert_eq!(backlog, VecDeque::from([b'/']));

        let mut backlog = VecDeque::from([b'x']);
        let mut after_cr = true;
        assert!(discard_form_lf_after_cr(&mut backlog, &mut after_cr));
        assert!(!after_cr);
        assert_eq!(backlog, VecDeque::from([b'x']));
    }

    #[test]
    fn test_invalid_form_line_discards_suffix_before_next_answer() {
        let mut backlog: VecDeque<u8> = b"\x1b/can".iter().copied().collect();
        let mut line = Vec::new();
        let mut after_cr = false;
        let mut echo_pending = Vec::new();
        assert!(matches!(
            consume_form_input(&mut backlog, &mut line, &mut after_cr, &mut echo_pending),
            FormInputProgress::Invalid { .. }
        ));
        assert!(!discard_form_input_line(&mut backlog, &mut after_cr));
        backlog.extend(b"cel\n/help\n".iter().copied());
        assert!(discard_form_input_line(&mut backlog, &mut after_cr));
        assert_eq!(backlog, b"/help\n".iter().copied().collect::<VecDeque<_>>());
        let FormInputProgress::Complete { line, .. } =
            consume_form_input(&mut backlog, &mut line, &mut after_cr, &mut echo_pending)
        else {
            panic!("the next line should remain available")
        };
        assert_eq!(line, b"/help");
    }

    #[tokio::test]
    async fn form_input_fetch_filters_startup_probe_replies() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut probe = Some(SshProbe {
            demux: ProbeDemux::new(),
            deadline: tokio::time::Instant::now() + Duration::from_secs(5),
        });
        // A cursor-position reply arrives first, then the user's answer.
        tx.send(SshInputFrame::Bytes(b"\x1b[1;1R".to_vec()))
            .await
            .unwrap();
        tx.send(SshInputFrame::Bytes(b"{\"action\":\"submit\"}\n".to_vec()))
            .await
            .unwrap();
        match recv_bytes_through_probe(&mut rx, &mut probe).await {
            FormBytes::Bytes(bytes) => {
                assert_eq!(bytes, b"{\"action\":\"submit\"}\n".to_vec())
            }
            _ => panic!("expected the filtered user bytes"),
        }
        assert!(
            probe.is_some(),
            "probe window stays open until its deadline"
        );

        // At the deadline the window closes and held partial-prefix
        // bytes are returned as input.
        let (tx2, mut rx2) = mpsc::channel(4);
        let mut probe2 = Some(SshProbe {
            demux: ProbeDemux::new(),
            deadline: tokio::time::Instant::now(),
        });
        tx2.send(SshInputFrame::Bytes(b"\x1b[".to_vec()))
            .await
            .unwrap();
        match recv_bytes_through_probe(&mut rx2, &mut probe2).await {
            FormBytes::Bytes(bytes) => assert_eq!(bytes, b"\x1b[".to_vec()),
            _ => panic!("expected held bytes at probe expiry"),
        }
        assert!(probe2.is_none());
    }

    #[test]
    fn test_terminal_form_presenter_renders_and_encodes_submit_and_cancel() {
        use crate::world_presentation::{FormAction, FormActionKind, FormField, FormSurface};

        let form = FormSurface {
            version: 1,
            form_id: "ssh-form".to_owned(),
            presentation_token: Some("0123456789abcdef0123456789abcdef".to_owned()),
            title: "Profile".to_owned(),
            fields: vec![
                FormField::Text {
                    id: "user_name".to_owned(),
                    label: "Name".to_owned(),
                    required: true,
                    max_length: 30,
                },
                FormField::Choice {
                    id: "tool".to_owned(),
                    label: "Tool".to_owned(),
                    required: true,
                    options: vec!["👩‍💻".to_owned()],
                },
            ],
            actions: vec![
                FormAction {
                    id: "submit".to_owned(),
                    label: "Save".to_owned(),
                    kind: FormActionKind::Submit,
                },
                FormAction {
                    id: "cancel".to_owned(),
                    label: "Cancel".to_owned(),
                    kind: FormActionKind::Cancel,
                },
            ],
        };
        let prompt = terminal_text(&terminal_form_prompt(&form));
        assert!(prompt.contains("Name — user_name [text; required=true; max_length=30]"));
        assert!(prompt.contains("\\u200d"));
        assert!(prompt.contains("cancel — Cancel (cancel)"));
        assert!(!prompt.contains('\u{001b}'));

        let submit = terminal_form_answer_frame(
            &form,
            r#"{"action":"submit","values":{"user_name":"Ada","tool":"👩‍💻"}}"#,
        )
        .unwrap();
        let submit: serde_json::Value = serde_json::from_str(&submit).unwrap();
        assert_eq!(submit["type"], "form_result");
        assert_eq!(submit["form_id"], "ssh-form");
        assert_eq!(
            submit["presentation_token"],
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(submit["action"], "submit");
        assert_eq!(submit["values"]["user_name"], "Ada");
        assert_eq!(submit["values"]["tool"], "👩‍💻");

        let cancel = terminal_form_answer_frame(&form, "/cancel").unwrap();
        let cancel: serde_json::Value = serde_json::from_str(&cancel).unwrap();
        assert_eq!(cancel["action"], "cancel");
        assert_eq!(cancel["values"], serde_json::json!({}));
    }

    #[test]
    fn test_form_input_preserves_packet_remainder_and_edits_utf8_characters() {
        let mut backlog: VecDeque<u8> = b"/cancel\n/help\n".iter().copied().collect();
        let mut line = Vec::new();
        let mut after_cr = false;
        let mut echo_pending = Vec::new();
        let FormInputProgress::Complete { line: first, .. } =
            consume_form_input(&mut backlog, &mut line, &mut after_cr, &mut echo_pending)
        else {
            panic!("first complete line should be returned")
        };
        assert_eq!(first, b"/cancel");
        let FormInputProgress::Complete { line: second, .. } =
            consume_form_input(&mut backlog, &mut line, &mut after_cr, &mut echo_pending)
        else {
            panic!("the packet remainder should stay available")
        };
        assert_eq!(second, b"/help");

        let mut backlog: VecDeque<u8> = "é\u{8}x\n".as_bytes().iter().copied().collect();
        let mut after_cr = false;
        let FormInputProgress::Complete { line, echo } = consume_form_input(
            &mut backlog,
            &mut Vec::new(),
            &mut after_cr,
            &mut echo_pending,
        ) else {
            panic!("edited UTF-8 line should be complete")
        };
        assert_eq!(line, b"x");
        assert_eq!(echo, "é\x08 \x08x\r\n".as_bytes());

        let mut backlog: VecDeque<u8> = b"a\xe2\x08\n".iter().copied().collect();
        let mut after_cr = false;
        let mut echo_pending = Vec::new();
        let FormInputProgress::Complete { line, .. } = consume_form_input(
            &mut backlog,
            &mut Vec::new(),
            &mut after_cr,
            &mut echo_pending,
        ) else {
            panic!("backspace should remove only the incomplete UTF-8 scalar")
        };
        assert_eq!(line, b"a");
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

    #[test]
    fn records_cursor_must_be_a_string_that_advances() {
        // Issue #6615: the SSH /records drain trusted `next_after`
        // unchecked — a non-string cursor silently read as a complete
        // listing, and a repeated/empty cursor re-requested the same
        // page ~18,700 times until the rendered-byte cap stopped it.
        // Oracle: the world_net records cursor contract (#6505/#6578)
        // — the cursor must be a string and must strictly advance.
        let cursor =
            |value: serde_json::Value, produced_by: &str| records_cursor(Some(&value), produced_by);

        // Absent and null both mean the host has no more pages.
        assert_eq!(records_cursor(None, ""), RecordsCursor::End);
        assert_eq!(cursor(serde_json::Value::Null, "k1"), RecordsCursor::End);

        // A normal multi-page drain: each cursor advances past the
        // one that produced the page, then the listing ends.
        assert_eq!(
            cursor(serde_json::json!("k1"), ""),
            RecordsCursor::Advance("k1".to_owned())
        );
        assert_eq!(
            cursor(serde_json::json!("k2"), "k1"),
            RecordsCursor::Advance("k2".to_owned())
        );
        assert_eq!(records_cursor(None, "k2"), RecordsCursor::End);

        // Present but not a string: a malformed page, never End.
        for bad in [
            serde_json::json!(123),
            serde_json::json!(true),
            serde_json::json!({"key": "k1"}),
            serde_json::json!(["k1"]),
        ] {
            assert_eq!(
                cursor(bad, ""),
                RecordsCursor::Malformed,
                "non-string cursor must be malformed"
            );
        }

        // A cursor that does not advance stops the drain: the empty
        // cursor on the first page, a repeat of the producing
        // cursor, and a step backwards.
        assert_eq!(cursor(serde_json::json!(""), ""), RecordsCursor::Stalled);
        assert_eq!(
            cursor(serde_json::json!("k1"), "k1"),
            RecordsCursor::Stalled
        );
        assert_eq!(
            cursor(serde_json::json!("k0"), "k1"),
            RecordsCursor::Stalled
        );
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
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
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
        // The initial PTY resize may still be queued after the first view and
        // correctly forces one more full repaint. Consume that recovery frame
        // before checking a stable, consecutive update.
        let _resize_recovery = read_until_bytes(&mut channel, &mut terminal, "count=1").await;
        channel.data(&b"inc"[..]).await.unwrap();
        let update = read_until_bytes(&mut channel, &mut terminal, "count=2").await;
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

    /// Proving test for tracker #796 item 9-6: an SSH `window-change`
    /// request must reach the session as a resize and refit the shell view
    /// to the new viewport size (drafted on #796 from task 6509; landed once
    /// the ssh feature compiled again after #3962).
    /// A program whose view is one fixed 4x2 ASCII screen.
    struct FixedScreenSsh;

    impl crate::world::WorldProgram for FixedScreenSsh {
        fn view(&mut self, _viewer: &crate::world::Participant) -> anyhow::Result<Option<String>> {
            Ok(Some(
                r#"{"kind":"screen","version":1,"width":4,"height":2,"rows":["abcd","wxyz"]}"#
                    .to_owned(),
            ))
        }
    }

    #[tokio::test]
    async fn a_window_change_refits_the_shell_view_to_the_new_size() {
        let world = WorldHandle::spawn_with_program(
            WorldCore::new("lobby", WorldLimits::default()).unwrap(),
            Box::new(FixedScreenSsh),
        );
        let (_, capability) = world
            .issue(
                "player".to_owned(),
                crate::world::WorldScopes::from_names(["join", "input"]).unwrap(),
            )
            .await
            .unwrap();
        let hub = WorldHub::single(WorldService::new(world, Some("admin".to_owned())));
        let (port, server) = start(hub).await;
        let session = connect(port, capability.expose()).await;
        let mut channel = shell(&session).await;
        let mut terminal = vt100::Parser::new(24, 80, 0);
        // The pty opens at 80x24, so the whole 4x2 screen is shown first.
        read_until(&mut channel, &mut terminal, "wxyz").await;
        assert!(terminal.screen().contents().contains("abcd"));

        // A terminal window change must reach the session as a resize and
        // refit the view: 2x1 keeps only the first two cells of row one.
        channel.window_change(2, 1, 0, 0).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let contents = terminal.screen().contents();
                if contents.contains("ab") && !contents.contains("wxyz") {
                    break;
                }
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => terminal.process(&data),
                    Some(_) => {}
                    None => panic!("shell closed before the refitted view"),
                }
            }
        })
        .await
        .expect("timed out waiting for the refitted view");
        let contents = terminal.screen().contents();
        assert!(contents.contains("ab"), "{contents:?}");
        assert!(!contents.contains("cd"), "{contents:?}");
        server.abort();
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_shell_plays_the_world_in_command_mode_using_input_command() {
        use iroh_blobs::store::mem::MemStore;

        let blobs: iroh_blobs::api::Store = MemStore::new().into();
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, Some("admin".to_owned())).with_blob_store(blobs);
        let wasm = include_bytes!("../../../examples/roc-counter/counter.wasm").to_vec();
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
