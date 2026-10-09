//! SSH presentation for a hosted world.
//!
//! The SSH user names the world (empty for the default world) and the SSH
//! password is the join capability. Keystrokes become `input` frames and each
//! `view` frame repaints the terminal, so an ordinary `ssh` client can play any
//! world that presents a view.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use russh::{
    Channel, ChannelId, ChannelOpenFailure, MethodKind, MethodSet, Pty,
    keys::{Algorithm, PrivateKey, ssh_key::HashAlg, ssh_key::LineEnding},
    server::{Auth, ChannelOpenHandle, Config, Handle, Handler, Msg, Server, Session},
};
use serde_json::json;
use tokio::{
    net::TcpListener,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};

use crate::directory_view::{DirectoryAction, render_text};
use crate::world_hub::WorldHub;
use crate::world_session::{Inbound, SessionIo, encode_hex, run_session};

/// File, beside the other server keys, that holds the SSH host key.
pub const SSH_HOST_KEY_FILE: &str = ".ssh-host-key";

const INPUT_QUEUE: usize = 256;
/// User-name prefix that opens the read-only directory explorer for a world.
const EXPLORE_PREFIX: &str = "explore:";
const MAX_PACKET_BYTES: u32 = 32 * 1024;
const INACTIVITY_TIMEOUT: Duration = Duration::from_mins(10);
/// Concurrent SSH connections the server accepts; the rest are refused at auth.
const MAX_CONNECTIONS: usize = 64;
/// Pause before a refused shell closes, so guessing costs at least this per connection.
const REFUSED_SHELL_DELAY: Duration = Duration::from_secs(1);

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
        methods: MethodSet::from(&[MethodKind::Password][..]),
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
            let world = login.world.clone();
            let password = login.capability.clone();
            let hub = self.hub.clone();
            let handle = session.handle();
            session.channel_success(channel)?;
            tokio::spawn(async move {
                let text = match explore(&hub, world.as_deref(), &password).await {
                    Ok(text) => text,
                    Err(message) => {
                        tokio::time::sleep(REFUSED_SHELL_DELAY).await;
                        format!("{message}\r\n")
                    }
                };
                let _ = handle.data(channel, text).await;
                let _ = handle.close(channel).await;
            });
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

/// The directory as text. The password is the admin token, an account credential, or empty.
async fn explore(hub: &WorldHub, world: Option<&str>, password: &str) -> Result<String, String> {
    let Ok(lease) = hub.lease(world, Some(password)).await else {
        return Err("no such world".to_owned());
    };
    let admin = hub.admin_ok(password);
    let credential = (!admin && !password.is_empty()).then(|| password.to_owned());
    let outcome = lease
        .service()
        .directory(credential, admin, DirectoryAction::View)
        .await
        .map_err(|error| format!("{error:#}"))?;
    Ok(render_text(&outcome.view).replace('\n', "\r\n"))
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
    use russh::{ChannelMsg, client};

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

    async fn read_all(channel: &mut Channel<client::Msg>) -> String {
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(message) = channel.wait().await {
                if let ChannelMsg::Data { data } = message {
                    bytes.extend_from_slice(&data);
                }
            }
        })
        .await
        .expect("timed out reading the explorer");
        String::from_utf8(bytes).unwrap()
    }

    #[tokio::test]
    async fn the_explorer_shows_the_directory_as_text_to_the_admin_and_anonymous() {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let signed = world
            .directory(
                None,
                false,
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
            let mut channel = session.channel_open_session().await.unwrap();
            channel.request_shell(true).await.unwrap();
            let text = read_all(&mut channel).await;
            assert!(text.contains(&ada) && text.contains("Ada"), "{text}");
            assert_eq!(text.contains("viewer: admin"), is_admin, "{text}");
        }
        server.abort();
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
