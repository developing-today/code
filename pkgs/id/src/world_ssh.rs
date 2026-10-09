//! SSH presentation for a hosted world.
//!
//! The SSH user names the world (empty for the default world) and the SSH
//! password is the join capability. Keystrokes become `input` frames and each
//! `view` frame repaints the terminal, so an ordinary `ssh` client can play any
//! world that presents a view.

use std::{fmt::Write as _, net::SocketAddr, sync::Arc, time::Duration};

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
use crate::world::hex_encode;
use crate::world_hub::{ResolveError, WorldHub};
use crate::world_session::{Inbound, SessionIo, encode_hex, run_session};

/// File, beside the other server keys, that holds the SSH host key.
pub const SSH_HOST_KEY_FILE: &str = ".ssh-host-key";

const INPUT_QUEUE: usize = 256;
/// User-name prefix that opens the directory explorer for a world.
const EXPLORE_PREFIX: &str = "explore:";
const MAX_PACKET_BYTES: u32 = 32 * 1024;
const INACTIVITY_TIMEOUT: Duration = Duration::from_mins(10);
/// Concurrent SSH connections the server accepts; the rest are refused at auth.
const MAX_CONNECTIONS: usize = 64;
/// Pause before a refused shell closes, so guessing costs at least this per connection.
const REFUSED_SHELL_DELAY: Duration = Duration::from_secs(1);
const PROMPT: &str = "id> ";
/// Longest line the explorer keeps; the rest of a longer line is dropped.
const MAX_LINE: usize = 1024;

/// Serve SSH connections until the listener fails.
pub async fn serve(
    listener: TcpListener,
    hub: WorldHub,
    host_key: PrivateKey,
) -> std::io::Result<()> {
    serve_limited(listener, hub, host_key, MAX_CONNECTIONS).await
}

async fn serve_limited(
    listener: TcpListener,
    hub: WorldHub,
    host_key: PrivateKey,
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
    connections: Arc<Semaphore>,
}

impl Server for SshServer {
    type Handler = SshClient;

    fn new_client(&mut self, _peer: Option<SocketAddr>) -> SshClient {
        SshClient {
            hub: self.hub.clone(),
            permit: Arc::clone(&self.connections).try_acquire_owned().ok(),
            login: None,
            input: None,
        }
    }
}

struct Login {
    world: Option<String>,
    capability: String,
    explore: bool,
    /// The public key the client proved, as hex. Only explorers have one.
    key: Option<String>,
}

struct SshClient {
    hub: WorldHub,
    permit: Option<OwnedSemaphorePermit>,
    login: Option<Login>,
    input: Option<mpsc::Sender<Vec<u8>>>,
}

impl Handler for SshClient {
    type Error = russh::Error;

    // The join capability is checked when the shell opens, so every password
    // is accepted here and a wrong one is answered by the world itself.
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, russh::Error> {
        if self.permit.is_none() {
            return Ok(Auth::reject());
        }
        let (world, explore) = match user.strip_prefix(EXPLORE_PREFIX) {
            Some(world) => (world, true),
            None => (user, false),
        };
        self.login = Some(Login {
            world: (!world.is_empty()).then(|| world.to_owned()),
            capability: password.to_owned(),
            explore,
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
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        session.channel_failure(channel)
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
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), russh::Error> {
        let Some(login) = self.login.as_ref().filter(|_| self.input.is_none()) else {
            return session.channel_failure(channel);
        };
        if login.explore {
            let explorer = Explorer::new(self.hub.clone(), login, &login.capability);
            let (input, bytes) = mpsc::channel(INPUT_QUEUE);
            self.input = Some(input);
            let handle = session.handle();
            session.channel_success(channel)?;
            tokio::spawn(run_explorer(explorer, handle, channel, bytes));
            return Ok(());
        }
        let first = join_frame(login);
        session.channel_success(channel)?;

        let (input, frames) = mpsc::channel(INPUT_QUEUE);
        self.input = Some(input);
        let mut io = SshIo {
            handle: session.handle(),
            channel,
            frames,
            first: Some(first),
            errored: false,
        };
        let hub = self.hub.clone();
        tokio::spawn(async move {
            run_session(&hub, &mut io).await;
            if io.errored {
                tokio::time::sleep(REFUSED_SHELL_DELAY).await;
            }
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
        let Some(input) = &self.input else {
            return Ok(());
        };
        // Never wait for queue space: this handler runs on the same loop that
        // flushes the world's output, so waiting here could deadlock.
        if let Err(error) = input.try_send(data.to_vec()) {
            match error {
                mpsc::error::TrySendError::Full(_) => {
                    tracing::debug!("world ssh input queue full; dropping input");
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
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), russh::Error> {
        self.input = None;
        Ok(())
    }
}

fn join_frame(login: &Login) -> String {
    let mut join = json!({ "type": "join", "capability": login.capability });
    if let Some(world) = &login.world {
        join["world"] = json!(world);
    }
    join.to_string()
}

/// [`SessionIo`] for one SSH shell: the join frame first, then keystrokes in,
/// and terminal output for each view out.
struct SshIo {
    handle: Handle,
    channel: ChannelId,
    frames: mpsc::Receiver<Vec<u8>>,
    first: Option<String>,
    errored: bool,
}

impl SessionIo for SshIo {
    async fn recv(&mut self) -> Inbound {
        if let Some(join) = self.first.take() {
            return Inbound::Text(join);
        }
        match self.frames.recv().await {
            Some(bytes) => Inbound::Text(
                json!({ "type": "input", "data_hex": encode_hex(&bytes) }).to_string(),
            ),
            None => Inbound::Closed,
        }
    }

    async fn send(&mut self, frame: String) -> bool {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&frame) else {
            return true;
        };
        let text = match value["type"].as_str() {
            Some("view") => match value["data"].as_str() {
                Some(data) => format!("\x1b[H\x1b[2J{}", terminal_text(data)),
                None => return true,
            },
            Some("error") => {
                self.errored = true;
                let message = value["message"].as_str().unwrap_or("error");
                format!("\r\n{}\r\n", terminal_text(message))
            }
            _ => return true,
        };
        self.handle.data(self.channel, text).await.is_ok()
    }
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
    mut bytes: mpsc::Receiver<Vec<u8>>,
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
    'input: while let Some(chunk) = bytes.recv().await {
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
        if !out.is_empty()
            && handle
                .data(channel, String::from_utf8_lossy(&out).into_owned())
                .await
                .is_err()
        {
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
        start_limited(hub, MAX_CONNECTIONS).await
    }

    async fn start_limited(
        hub: WorldHub,
        max_connections: usize,
    ) -> (u16, tokio::task::JoinHandle<std::io::Result<()>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        (
            port,
            tokio::spawn(serve_limited(listener, hub, host_key, max_connections)),
        )
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
        tokio::time::timeout(Duration::from_secs(10), async {
            while !terminal.screen().contents().contains(needle) {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => terminal.process(&data),
                    Some(_) => {}
                    None => panic!("shell closed before {needle:?}"),
                }
            }
        })
        .await
        .expect("timed out waiting for the terminal");
    }

    fn lobby_hub() -> WorldHub {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        WorldHub::single(WorldService::new(world, Some("admin".to_owned())))
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
    async fn connections_over_the_limit_are_refused_at_auth() {
        let (port, server) = start_limited(lobby_hub(), 1).await;

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
    fn terminal_text_drops_control_characters_but_keeps_lines() {
        assert_eq!(
            terminal_text("a\x1b]52;c;AAAA\x07b\r\nc\td"),
            "a]52;c;AAAAb\r\nc\td"
        );
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_shell_plays_the_world_and_repaints_each_view() {
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
            .invite(Some("admin"), "ssh".to_owned())
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
        read_until(&mut channel, &mut terminal, "count=0").await;
        channel.data(&b"inc"[..]).await.unwrap();
        read_until(&mut channel, &mut terminal, "count=1").await;
        server.abort();
    }
}
