//! Transport-neutral world session driver.
//!
//! A *session* is one participant's connection to a [`WorldHandle`]. The
//! wire protocol is one JSON object per frame; this module owns that protocol
//! and nothing else. A transport (WebSocket, Iroh stream, an in-memory test
//! channel) only implements [`SessionIo`] to move whole text frames, so the
//! behavior cannot drift between transports.
//!
//! Client frames (`type` tag): `join {capability, after?}`,
//! `invite {admin_token, display_name}`, `chat {text}`, `input {data_hex}`.
//! The first frame must be `join` or `invite`. Server frames: `snapshot`,
//! `event`, `invite`, `error`.
//!
//! Authorization is the capability alone. A transport may know who the peer
//! is (an Iroh node ID); that never grants world authority.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use tokio::sync::broadcast;

use crate::world::{JoinCapability, WorldEvent, WorldHandle, WorldScopes, WorldSnapshot};

/// Largest accepted or emitted frame, in bytes.
pub const MAX_FRAME_BYTES: usize = 16 * 1024;

/// Longest accepted capability string, in bytes.
const MAX_CAPABILITY_BYTES: usize = 256;

/// Longest accepted admin secret, in bytes.
const MAX_ADMIN_TOKEN_BYTES: usize = 256;

/// One inbound unit from a transport.
#[derive(Debug)]
pub enum Inbound {
    /// A complete text frame no larger than [`MAX_FRAME_BYTES`].
    Text(String),
    /// A frame larger than [`MAX_FRAME_BYTES`] (its contents are discarded).
    Oversized,
    /// A frame that is not UTF-8 text.
    Unsupported,
    /// The peer is gone or the stream failed; the session must end.
    Closed,
}

/// Moves whole text frames for one session.
///
/// `recv` must be cancel-safe: the driver races it against world events and a
/// cancelled call must not lose or split a frame.
pub trait SessionIo: Send {
    /// Next inbound frame.
    fn recv(&mut self) -> impl Future<Output = Inbound> + Send;
    /// Send one frame. Returns `false` when the peer is gone.
    fn send(&mut self, frame: String) -> impl Future<Output = bool> + Send;
}

/// Tunables for [`run_session_with`].
#[derive(Clone, Copy, Debug)]
pub struct SessionConfig {
    /// How long a peer may take to send its first frame.
    pub join_timeout: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            join_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Join {
        capability: String,
        after: Option<u64>,
    },
    Invite {
        admin_token: String,
        display_name: String,
    },
    Chat {
        text: String,
    },
    Input {
        data_hex: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame<'a> {
    Snapshot { snapshot: &'a WorldSnapshot },
    Event { event: &'a WorldEvent },
    Invite(&'a InviteResponse),
    Error { message: &'a str },
}

/// A freshly minted guest capability.
#[derive(Debug, Serialize)]
pub struct InviteResponse {
    /// Bearer capability; shown once, never logged.
    pub capability: String,
    /// The new participant's ID.
    pub participant_id: u64,
    /// The sanitized display name.
    pub display_name: String,
}

/// Why an invite was not issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteError {
    /// Invites are not enabled (no admin secret configured).
    Disabled,
    /// The supplied admin secret is missing or wrong.
    Unauthorized,
    /// The world refused another participant (full, or shut down).
    Unavailable,
}

/// A world plus the secret that may mint guest capabilities for it.
#[derive(Clone)]
pub struct WorldService {
    world: WorldHandle,
    admin_token: Option<String>,
}

impl std::fmt::Debug for WorldService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldService")
            .field(
                "admin_token",
                &self.admin_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish_non_exhaustive()
    }
}

impl WorldService {
    /// Wrap a world. `admin_token: None` disables invites.
    #[must_use]
    pub const fn new(world: WorldHandle, admin_token: Option<String>) -> Self {
        Self { world, admin_token }
    }

    /// The underlying world actor.
    #[must_use]
    pub const fn world(&self) -> &WorldHandle {
        &self.world
    }

    /// Mint a guest capability if `supplied` matches the admin secret.
    ///
    /// # Errors
    ///
    /// See [`InviteError`].
    pub async fn invite(
        &self,
        supplied: Option<&str>,
        display_name: String,
    ) -> Result<InviteResponse, InviteError> {
        let Some(expected) = self.admin_token.as_deref().filter(|t| !t.is_empty()) else {
            return Err(InviteError::Disabled);
        };
        if !supplied.is_some_and(|s| {
            s.len() <= MAX_ADMIN_TOKEN_BYTES && secret_eq(expected.as_bytes(), s.as_bytes())
        }) {
            return Err(InviteError::Unauthorized);
        }
        match self.world.issue(display_name, WorldScopes::GUEST).await {
            Ok((participant, capability)) => Ok(InviteResponse {
                capability: capability.expose().to_owned(),
                participant_id: participant.id,
                display_name: participant.display_name,
            }),
            Err(_) => Err(InviteError::Unavailable),
        }
    }
}

/// Constant-time (for equal lengths) secret comparison.
#[must_use]
pub fn secret_eq(expected: &[u8], supplied: &[u8]) -> bool {
    expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
}

/// Run one session to completion with default settings.
pub async fn run_session<I: SessionIo>(service: &WorldService, io: &mut I) {
    run_session_with(service, io, SessionConfig::default()).await;
}

/// Run one session to completion.
pub async fn run_session_with<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    config: SessionConfig,
) {
    let first = match tokio::time::timeout(config.join_timeout, io.recv()).await {
        Ok(Inbound::Text(text)) => text,
        Ok(Inbound::Oversized) => {
            let _ = send_error(io, "world frame is too large").await;
            return;
        }
        Ok(Inbound::Unsupported) => {
            let _ = send_error(io, "frames must be UTF-8 text").await;
            return;
        }
        Ok(Inbound::Closed) => return,
        Err(_) => {
            let _ = send_error(io, "timed out waiting for join").await;
            return;
        }
    };
    let Ok(frame) = serde_json::from_str::<ClientFrame>(&first) else {
        let _ = send_error(io, "first frame must be a valid join or invite request").await;
        return;
    };
    match frame {
        ClientFrame::Join { capability, after } => {
            if capability.len() > MAX_CAPABILITY_BYTES {
                let _ = send_error(io, "invalid join capability").await;
                return;
            }
            joined(
                service.world(),
                io,
                JoinCapability::from_wire(capability),
                after,
            )
            .await;
        }
        ClientFrame::Invite {
            admin_token,
            display_name,
        } => match service.invite(Some(&admin_token), display_name).await {
            Ok(invite) => {
                let _ = send_json(io, &ServerFrame::Invite(&invite)).await;
            }
            Err(InviteError::Disabled) => {
                let _ = send_error(io, "invites are not enabled").await;
            }
            Err(InviteError::Unauthorized) => {
                let _ = send_error(io, "invite denied").await;
            }
            Err(InviteError::Unavailable) => {
                let _ = send_error(io, "world cannot issue another invite").await;
            }
        },
        ClientFrame::Chat { .. } | ClientFrame::Input { .. } => {
            let _ = send_error(io, "first frame must be a join request").await;
        }
    }
}

async fn joined<I: SessionIo>(
    world: &WorldHandle,
    io: &mut I,
    token: JoinCapability,
    after: Option<u64>,
) {
    // Subscribe before taking the snapshot so no event can fall in the gap;
    // events at or below the cursor are skipped when delivered.
    let mut events = world.subscribe();
    let Ok(snapshot) = world.snapshot(token.clone()).await else {
        let _ = send_error(io, "join denied").await;
        return;
    };
    let mut cursor = snapshot.current_sequence;
    if let Some(after) = after {
        match world.events_after(token.clone(), after).await {
            Ok(page) if page.needs_snapshot => {
                let Ok(fresh) = world.snapshot(token.clone()).await else {
                    return;
                };
                cursor = fresh.current_sequence;
                if !send_json(io, &ServerFrame::Snapshot { snapshot: &fresh }).await {
                    return;
                }
            }
            Ok(page) => {
                cursor = page.current_sequence;
                for event in &page.events {
                    if !send_json(io, &ServerFrame::Event { event }).await {
                        return;
                    }
                }
            }
            Err(_) => {
                let _ = send_error(io, "join denied").await;
                return;
            }
        }
    } else if !send_json(
        io,
        &ServerFrame::Snapshot {
            snapshot: &snapshot,
        },
    )
    .await
    {
        return;
    }

    loop {
        tokio::select! {
            inbound = io.recv() => match inbound {
                Inbound::Text(text) => {
                    if !handle_client_frame(world, io, &token, &text).await {
                        break;
                    }
                }
                Inbound::Oversized => {
                    if !send_error(io, "world frame is too large").await { break; }
                }
                Inbound::Unsupported => {
                    if !send_error(io, "frames must be UTF-8 text").await { break; }
                }
                Inbound::Closed => break,
            },
            event = events.recv() => match event {
                Ok(event) => {
                    if event.sequence <= cursor {
                        continue;
                    }
                    // Re-validating the capability on every delivery is what
                    // disconnects a revoked participant.
                    if world.snapshot(token.clone()).await.is_err() {
                        break;
                    }
                    if !send_json(io, &ServerFrame::Event { event: &event }).await {
                        break;
                    }
                    cursor = event.sequence;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let Ok(snapshot) = world.snapshot(token.clone()).await else { break };
                    if !send_json(io, &ServerFrame::Snapshot { snapshot: &snapshot }).await {
                        break;
                    }
                    cursor = snapshot.current_sequence;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
}

/// Handle one frame from a joined participant. Returns `false` to end the session.
async fn handle_client_frame<I: SessionIo>(
    world: &WorldHandle,
    io: &mut I,
    token: &JoinCapability,
    text: &str,
) -> bool {
    let Ok(frame) = serde_json::from_str::<ClientFrame>(text) else {
        return send_error(io, "invalid world frame").await;
    };
    let result = match frame {
        ClientFrame::Chat { text } => world.chat(token.clone(), text).await,
        ClientFrame::Input { data_hex } => match decode_hex(&data_hex) {
            Some(data) => world.input(token.clone(), data).await,
            None => return send_error(io, "input data must be hexadecimal").await,
        },
        ClientFrame::Join { .. } | ClientFrame::Invite { .. } => {
            return send_error(io, "already joined").await;
        }
    };
    // Accepted events reach everyone, including the sender, through the
    // broadcast branch, so only rejections are answered directly.
    result.is_ok() || send_error(io, "world event rejected").await
}

async fn send_json<I: SessionIo>(io: &mut I, frame: &impl Serialize) -> bool {
    match serde_json::to_string(frame) {
        Ok(encoded) if encoded.len() <= MAX_FRAME_BYTES * 4 => io.send(encoded).await,
        _ => false,
    }
}

async fn send_error<I: SessionIo>(io: &mut I, message: &str) -> bool {
    send_json(io, &ServerFrame::Error { message }).await
}

/// Decode a bounded hex string.
#[must_use]
pub fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) || encoded.len() > MAX_FRAME_BYTES * 2 {
        return None;
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::world::{WorldCore, WorldLimits};

    /// In-memory transport: tests push frames in and read frames out.
    struct ChannelIo {
        inbound: mpsc::Receiver<Inbound>,
        outbound: mpsc::Sender<String>,
    }

    impl SessionIo for ChannelIo {
        async fn recv(&mut self) -> Inbound {
            self.inbound.recv().await.unwrap_or(Inbound::Closed)
        }
        async fn send(&mut self, frame: String) -> bool {
            self.outbound.send(frame).await.is_ok()
        }
    }

    struct Peer {
        to_server: mpsc::Sender<Inbound>,
        from_server: mpsc::Receiver<String>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Peer {
        fn connect(service: WorldService, config: SessionConfig) -> Self {
            let (to_server, inbound) = mpsc::channel(16);
            let (outbound, from_server) = mpsc::channel(64);
            let task = tokio::spawn(async move {
                let mut io = ChannelIo { inbound, outbound };
                run_session_with(&service, &mut io, config).await;
            });
            Self {
                to_server,
                from_server,
                task,
            }
        }

        async fn say(&self, frame: serde_json::Value) {
            self.to_server
                .send(Inbound::Text(frame.to_string()))
                .await
                .unwrap();
        }

        async fn next(&mut self) -> Option<serde_json::Value> {
            let frame = tokio::time::timeout(Duration::from_secs(5), self.from_server.recv())
                .await
                .expect("timed out waiting for a server frame")?;
            Some(serde_json::from_str(&frame).unwrap())
        }
    }

    fn service(admin: Option<&str>) -> WorldService {
        WorldService::new(
            WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap()),
            admin.map(str::to_owned),
        )
    }

    #[test]
    fn hex_input_parser_is_bounded_and_strict() {
        assert_eq!(decode_hex("00aBff"), Some(vec![0, 0xab, 0xff]));
        assert_eq!(decode_hex(""), Some(vec![]));
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("zz"), None);
        assert_eq!(decode_hex(&"0".repeat(MAX_FRAME_BYTES * 2 + 2)), None);
    }

    #[test]
    fn wire_event_shape_is_presentation_neutral() {
        use crate::world::WorldEventKind;
        let event = WorldEvent {
            sequence: 3,
            participant_id: 9,
            kind: WorldEventKind::Chat("hello".to_owned()),
        };
        let encoded = serde_json::to_string(&ServerFrame::Event { event: &event }).unwrap();
        assert!(encoded.contains("\"type\":\"event\""));
        assert!(encoded.contains("\"sequence\":3"));
        assert!(encoded.contains("\"kind\":\"chat\""));
        assert!(encoded.contains("\"data\":\"hello\""));
    }

    #[test]
    fn secret_comparison_requires_equal_bytes() {
        assert!(secret_eq(b"abc", b"abc"));
        assert!(!secret_eq(b"abc", b"abd"));
        assert!(!secret_eq(b"abc", b"ab"));
    }

    #[tokio::test]
    async fn silent_peers_are_dropped_after_the_join_timeout() {
        let mut peer = Peer::connect(
            service(None),
            SessionConfig {
                join_timeout: Duration::from_millis(50),
            },
        );
        let frame = peer.next().await.unwrap();
        assert_eq!(frame["type"], "error");
        assert!(frame["message"].as_str().unwrap().contains("timed out"));
        assert!(peer.next().await.is_none());
        peer.task.await.unwrap();
    }

    #[tokio::test]
    async fn first_frame_must_be_join_or_invite() {
        let mut peer = Peer::connect(service(Some("a")), SessionConfig::default());
        peer.say(serde_json::json!({"type": "chat", "text": "hi"}))
            .await;
        assert_eq!(peer.next().await.unwrap()["type"], "error");
        assert!(peer.next().await.is_none());

        let mut garbage = Peer::connect(service(None), SessionConfig::default());
        garbage
            .to_server
            .send(Inbound::Text("not json".to_owned()))
            .await
            .unwrap();
        assert_eq!(garbage.next().await.unwrap()["type"], "error");

        let mut big = Peer::connect(service(None), SessionConfig::default());
        big.to_server.send(Inbound::Oversized).await.unwrap();
        assert_eq!(big.next().await.unwrap()["type"], "error");
        let mut bin = Peer::connect(service(None), SessionConfig::default());
        bin.to_server.send(Inbound::Unsupported).await.unwrap();
        assert_eq!(bin.next().await.unwrap()["type"], "error");
    }

    #[tokio::test]
    async fn invite_frame_issues_a_capability_that_can_join() {
        let svc = service(Some("admin"));

        let mut denied = Peer::connect(svc.clone(), SessionConfig::default());
        denied
            .say(serde_json::json!({"type":"invite","admin_token":"nope","display_name":"x"}))
            .await;
        assert_eq!(denied.next().await.unwrap()["message"], "invite denied");

        let mut host = Peer::connect(svc.clone(), SessionConfig::default());
        host.say(serde_json::json!({"type":"invite","admin_token":"admin","display_name":"ann"}))
            .await;
        let invite = host.next().await.unwrap();
        assert_eq!(invite["type"], "invite");
        let capability = invite["capability"].as_str().unwrap().to_owned();

        let mut guest = Peer::connect(svc, SessionConfig::default());
        guest
            .say(serde_json::json!({"type":"join","capability":capability}))
            .await;
        assert_eq!(guest.next().await.unwrap()["type"], "snapshot");
    }

    #[tokio::test]
    async fn invites_are_disabled_without_an_admin_secret() {
        let mut peer = Peer::connect(service(None), SessionConfig::default());
        peer.say(serde_json::json!({"type":"invite","admin_token":"","display_name":"x"}))
            .await;
        assert_eq!(
            peer.next().await.unwrap()["message"],
            "invites are not enabled"
        );
    }

    #[tokio::test]
    async fn chat_is_broadcast_and_rejections_are_private() {
        let svc = service(None);
        let (_, alice) = svc
            .world()
            .issue("alice".to_owned(), WorldScopes::GUEST)
            .await
            .unwrap();
        let (_, watcher) = svc
            .world()
            .issue("watcher".to_owned(), WorldScopes::JOIN)
            .await
            .unwrap();
        let mut a = Peer::connect(svc.clone(), SessionConfig::default());
        let mut w = Peer::connect(svc, SessionConfig::default());
        a.say(serde_json::json!({"type":"join","capability":alice.expose()}))
            .await;
        w.say(serde_json::json!({"type":"join","capability":watcher.expose()}))
            .await;
        assert_eq!(a.next().await.unwrap()["type"], "snapshot");
        assert_eq!(w.next().await.unwrap()["type"], "snapshot");

        // A join-only participant cannot speak; only they hear about it.
        w.say(serde_json::json!({"type":"chat","text":"psst"}))
            .await;
        assert_eq!(w.next().await.unwrap()["message"], "world event rejected");

        a.say(serde_json::json!({"type":"chat","text":"hello"}))
            .await;
        for peer in [&mut a, &mut w] {
            let frame = peer.next().await.unwrap();
            assert_eq!(frame["type"], "event");
            assert!(frame.to_string().contains("hello"));
        }
        a.say(serde_json::json!({"type":"input","data_hex":"xyz"}))
            .await;
        assert_eq!(
            a.next().await.unwrap()["message"],
            "input data must be hexadecimal"
        );
        a.say(serde_json::json!({"type":"join","capability":"x"}))
            .await;
        assert_eq!(a.next().await.unwrap()["message"], "already joined");
    }
}
