//! Iroh transport for friend envelopes (`/id-envelope/1`) and signed artifacts
//! (`/id-artifact-push/1`).
//!
//! One bidirectional stream carries one payload: the sender writes a frame
//! holding the payload's JSON, and the receiver answers with a frame holding a
//! [`Reply`]. The payload's signature is what makes it trusted, not the iroh
//! node that delivered it.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use serde::{Deserialize, Serialize};

use crate::connectivity::connect;
use crate::directory::{Envelope, Received, Refusal};
use crate::directory_auth::Caller;
use crate::directory_view::{DirectoryAction, DirectoryOutcome};
use crate::envelope_outbox::Outcome;
use crate::frames::{read_frame, write_frame};
use crate::world_hub::WorldHub;

/// ALPN for friend envelopes. Within version 1 fields may be added; removing or
/// changing one needs `/id-envelope/2`.
pub const ENVELOPE_ALPN: &[u8] = b"/id-envelope/1";

const TIMEOUT: Duration = Duration::from_secs(30);

/// The answer to one frame. `Refused` will never succeed; `Retry` may.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    /// The frame was applied.
    Applied,
    /// The frame was already applied.
    Duplicate,
    /// The frame is invalid and must not be sent again.
    Refused {
        /// Why it was refused.
        message: String,
    },
    /// The frame could not be applied now but may be later.
    Retry {
        /// Why it could not be applied now.
        message: String,
    },
}

/// Sends `envelope` to the node at `target`, and says whether it is done.
pub async fn deliver(
    endpoint: &Endpoint,
    target: impl Into<EndpointAddr>,
    envelope: &Envelope,
) -> Outcome {
    let json = match serde_json::to_vec(envelope) {
        Ok(json) => json,
        Err(error) => return Outcome::Refused(format!("envelope does not encode: {error}")),
    };
    send(endpoint, ENVELOPE_ALPN, target.into(), &json).await
}

/// Sends one frame on `alpn` to `target`, and says whether the peer took it.
/// The whole delivery, address lookup included, must finish within [`TIMEOUT`].
pub async fn send(endpoint: &Endpoint, alpn: &[u8], target: EndpointAddr, json: &[u8]) -> Outcome {
    send_within(endpoint, alpn, target, json, TIMEOUT).await
}

/// [`send`] with `limit` on the whole delivery.
pub async fn send_within(
    endpoint: &Endpoint,
    alpn: &[u8],
    target: EndpointAddr,
    json: &[u8],
    limit: Duration,
) -> Outcome {
    match tokio::time::timeout(limit, exchange(endpoint, alpn, target, json)).await {
        Ok(Ok(Reply::Applied | Reply::Duplicate)) => Outcome::Delivered,
        Ok(Ok(Reply::Refused { message })) => Outcome::Refused(message),
        Ok(Ok(Reply::Retry { message })) => Outcome::Retry(message),
        Ok(Err(error)) => Outcome::Retry(format!("{error:#}")),
        Err(_) => Outcome::Retry("delivery timed out".to_owned()),
    }
}

async fn exchange(
    endpoint: &Endpoint,
    alpn: &[u8],
    target: EndpointAddr,
    json: &[u8],
) -> Result<Reply> {
    let conn = connect(endpoint, target, alpn)
        .await
        .context("connecting to the recipient's server")?;
    let reply = async {
        let (mut send, mut recv) = conn.open_bi().await.context("opening a stream")?;
        write_frame(&mut send, json).await?;
        send.finish().context("finishing the request")?;
        let frame = read_frame(&mut recv).await?.context("no reply")?;
        serde_json::from_slice::<Reply>(&frame).context("malformed reply")
    }
    .await;
    conn.close(0_u32.into(), b"done");
    reply
}

/// Accepts friend envelopes over Iroh. Register with
/// `Router::builder(..).accept(ENVELOPE_ALPN, EnvelopeProtocol::new(hub))`.
#[derive(Clone, Debug)]
pub struct EnvelopeProtocol {
    hub: WorldHub,
}

impl EnvelopeProtocol {
    /// Serve the friend envelopes of every world in `hub`.
    #[must_use]
    pub const fn new(hub: WorldHub) -> Self {
        Self { hub }
    }
}

impl ProtocolHandler for EnvelopeProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let hub = self.hub.clone();
        answer_frames(&conn, move |body| {
            let hub = hub.clone();
            async move { receive(&hub, &body).await }
        })
        .await
    }
}

/// Answers frames sequentially on `conn` with the [`Reply`] `receive` gives it.
/// Each stream is bounded by [`TIMEOUT`]; a timed-out stream is reset and the
/// accept loop advances to the next stream.
pub async fn answer_frames<F, Fut>(conn: &Connection, receive: F) -> Result<(), AcceptError>
where
    F: Fn(Vec<u8>) -> Fut,
    Fut: Future<Output = Reply> + Send,
{
    answer_frames_within(conn, receive, TIMEOUT).await
}

async fn answer_frames_within<F, Fut>(
    conn: &Connection,
    receive: F,
    deadline: Duration,
) -> Result<(), AcceptError>
where
    F: Fn(Vec<u8>) -> Fut,
    Fut: Future<Output = Reply> + Send,
{
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let timed_out = tokio::time::timeout(deadline, async {
            let reply = match read_frame(&mut recv).await {
                Ok(Some(body)) => receive(body).await,
                Ok(None) => return,
                Err(error) => Reply::Refused {
                    message: format!("{error:#}"),
                },
            };
            let Ok(json) = serde_json::to_vec(&reply) else {
                return;
            };
            if write_frame(&mut send, &json).await.is_ok() {
                let _ = send.finish();
            }
        })
        .await
        .is_err();
        if timed_out {
            // `read_frame` is not cancel-safe. Abandon both halves rather
            // than attempting to reuse a partially consumed stream.
            let _ = send.reset(0_u32.into());
            let _ = recv.stop(0_u32.into());
        }
    }
    Ok(())
}

async fn receive(hub: &WorldHub, body: &[u8]) -> Reply {
    let envelope: Envelope = match serde_json::from_slice(body) {
        Ok(envelope) => envelope,
        Err(error) => {
            return Reply::Refused {
                message: format!("not an envelope: {error}"),
            };
        }
    };
    let Ok(lease) = hub.lease(None, None).await else {
        return Reply::Retry {
            message: "the server is busy; try again".to_owned(),
        };
    };
    match lease
        .service()
        .directory(Caller::default(), DirectoryAction::Receive { envelope })
        .await
    {
        Ok(DirectoryOutcome {
            received: Some(Received::Applied(_)),
            ..
        }) => Reply::Applied,
        Ok(DirectoryOutcome {
            received: Some(Received::Duplicate),
            ..
        }) => Reply::Duplicate,
        Ok(_) => Reply::Retry {
            message: "envelope was not received".to_owned(),
        },
        Err(error) if error.downcast_ref::<Refusal>().is_some() => Reply::Refused {
            message: format!("{error:#}"),
        },
        Err(error) => Reply::Retry {
            message: format!("{error:#}"),
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use iroh::{
        address_lookup::MemoryLookup,
        endpoint::{RelayMode, presets},
        protocol::Router,
    };
    use iroh_blobs::{Hash, store::mem::MemStore};
    use tokio::sync::Mutex;

    use super::*;
    use crate::commands::put::add_raw;
    use crate::connectivity::silent::{
        endpoint_with_silent_lookup, unanswered_addr, unanswered_id,
    };
    use crate::directory::Directory;
    use crate::envelope_outbox::{Finish, Outbox, flush};
    use crate::world::{WorldCore, WorldHandle, WorldLimits};
    use crate::world_session::WorldService;

    const NOW: u64 = 1_000_000_000;

    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }

    async fn endpoint_with(lookup: &MemoryLookup) -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .address_lookup(lookup.clone())
            .bind()
            .await
            .unwrap()
    }

    fn loopback_addr(ep: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|addr| {
                let ip = if addr.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    addr.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, addr.port()))
            }),
        )
    }

    /// Test-only receiver for exercising the raw frame transport. This is not
    /// the production [`EnvelopeProtocol`], which accepts directory envelopes
    /// and deliberately refuses chat-frame JSON.
    #[derive(Clone, Debug)]
    struct CaptureFrames(Arc<Mutex<Vec<Vec<u8>>>>);

    impl ProtocolHandler for CaptureFrames {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            let frames = self.0.clone();
            answer_frames(&conn, move |body| {
                let frames = frames.clone();
                async move {
                    frames.lock().await.push(body);
                    Reply::Applied
                }
            })
            .await
        }
    }

    #[derive(Clone, Debug)]
    struct DeadlineCaptureFrames {
        frames: Arc<Mutex<Vec<Vec<u8>>>>,
        deadline: Duration,
    }

    impl ProtocolHandler for DeadlineCaptureFrames {
        async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
            let frames = self.frames.clone();
            answer_frames_within(
                &conn,
                move |body| {
                    let frames = frames.clone();
                    async move {
                        frames.lock().await.push(body);
                        Reply::Applied
                    }
                },
                self.deadline,
            )
            .await
        }
    }

    /// A lobby on a node that is reachable only over loopback.
    async fn lobby(ep: &Endpoint) -> (Router, EndpointAddr, String) {
        let world = WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, None);
        let bo = service
            .directory(
                Caller::default(),
                DirectoryAction::SignUp {
                    name: "Bo".to_owned(),
                },
            )
            .await
            .unwrap()
            .view
            .viewer;
        let router = Router::builder(ep.clone())
            .accept(
                ENVELOPE_ALPN,
                EnvelopeProtocol::new(WorldHub::single(service)),
            )
            .spawn();
        let addr = EndpointAddr::from_parts(
            ep.id(),
            ep.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    std::net::Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                iroh::TransportAddr::Ip(std::net::SocketAddr::new(ip, a.port()))
            }),
        );
        (router, addr, bo)
    }

    fn sent_from_home(to: &str, audience: &str, at: u64) -> Envelope {
        let mut home = Directory::new();
        let (credential, _) = home.sign_up("Ann").unwrap();
        home.request_friend(&credential, to, audience, at)
            .unwrap()
            .0
    }

    #[tokio::test]
    async fn an_envelope_is_applied_once_and_refusals_are_permanent() {
        let server = endpoint().await;
        let (_router, addr, bo) = lobby(&server).await;
        let client = endpoint().await;
        let at = crate::time::unix_ms();
        let envelope = sent_from_home(&bo, "lobby", at);

        assert_eq!(
            deliver(&client, addr.clone(), &envelope).await,
            Outcome::Delivered
        );
        assert_eq!(
            deliver(&client, addr.clone(), &envelope).await,
            Outcome::Delivered,
            "a duplicate is already applied, so it counts as delivered"
        );

        let elsewhere = sent_from_home(&bo, "elsewhere", at);
        assert!(matches!(
            deliver(&client, addr.clone(), &elsewhere).await,
            Outcome::Refused(_)
        ));

        let mut forged = sent_from_home(&bo, "lobby", at);
        forged.at = at + 1;
        assert!(matches!(
            deliver(&client, addr.clone(), &forged).await,
            Outcome::Refused(_)
        ));

        let stranger = sent_from_home(&"ab".repeat(32), "lobby", at);
        assert!(matches!(
            deliver(&client, addr, &stranger).await,
            Outcome::Refused(_)
        ));
    }

    /// A throwaway transport fixture for the proposed chat frame. It proves
    /// byte-preserving exchange on the existing ALPN and a local content-hash
    /// hit; it does not test CLI routing, production envelope acceptance, or
    /// chat signature verification.
    #[tokio::test]
    async fn test_review_gated_chat_frame_round_trips_and_resolves_local_blob() {
        let alice = endpoint().await;
        let bob = endpoint().await;
        let alice_frames = Arc::new(Mutex::new(Vec::new()));
        let bob_frames = Arc::new(Mutex::new(Vec::new()));
        let alice_router = Router::builder(alice.clone())
            .accept(ENVELOPE_ALPN, CaptureFrames(alice_frames.clone()))
            .spawn();
        let bob_router = Router::builder(bob.clone())
            .accept(ENVELOPE_ALPN, CaptureFrames(bob_frames.clone()))
            .spawn();

        let receiver_store: iroh_blobs::api::Store = MemStore::new().into();
        let blob_bytes = b"review-gated chat PoC local blob";
        let (blob_hash, _keep) = add_raw(&receiver_store, blob_bytes.to_vec()).await.unwrap();

        let frame_alice_to_bob = serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "channel": "group:local-poc",
            "author": alice.id().to_string(),
            "seq": 1,
            "after": [],
            "text": "blob link fixture",
            "links": [{
                "kind": "blob",
                "hash": blob_hash.to_string(),
                "author": alice.id().to_string()
            }],
            "sig": "test-only; no signature verification"
        }))
        .unwrap();
        let frame_bob_to_alice = serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "channel": "group:local-poc",
            "author": bob.id().to_string(),
            "seq": 1,
            "after": [],
            "text": "acknowledged",
            "links": [],
            "sig": "test-only; no signature verification"
        }))
        .unwrap();

        println!(
            "Alice -> Bob /id-envelope/1 frame bytes (UTF-8): {}",
            String::from_utf8(frame_alice_to_bob.clone()).unwrap()
        );
        println!(
            "Bob -> Alice /id-envelope/1 frame bytes (UTF-8): {}",
            String::from_utf8(frame_bob_to_alice.clone()).unwrap()
        );

        assert_eq!(
            send_within(
                &alice,
                ENVELOPE_ALPN,
                loopback_addr(&bob),
                &frame_alice_to_bob,
                Duration::from_secs(5),
            )
            .await,
            Outcome::Delivered
        );
        assert_eq!(
            send_within(
                &bob,
                ENVELOPE_ALPN,
                loopback_addr(&alice),
                &frame_bob_to_alice,
                Duration::from_secs(5),
            )
            .await,
            Outcome::Delivered
        );

        assert_eq!(
            bob_frames.lock().await.as_slice(),
            &[frame_alice_to_bob.clone()]
        );
        assert_eq!(
            alice_frames.lock().await.as_slice(),
            &[frame_bob_to_alice.clone()]
        );

        let received_alice_to_bob = bob_frames.lock().await[0].clone();
        let received_frame: serde_json::Value =
            serde_json::from_slice(&received_alice_to_bob).unwrap();
        let linked_hash: Hash = received_frame["links"][0]["hash"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(linked_hash, blob_hash);
        assert!(receiver_store.blobs().has(linked_hash).await.unwrap());
        let resolved = receiver_store.blobs().get_bytes(linked_hash).await.unwrap();
        assert_eq!(&*resolved, blob_bytes);
        assert_eq!(Hash::new(&*resolved), blob_hash);
        println!(
            "Bob local blob resolution: hash={blob_hash} bytes={} hash_verified=true",
            resolved.len()
        );

        alice.close().await;
        bob.close().await;
        alice_router.shutdown().await.unwrap();
        bob_router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn an_unreachable_recipient_is_retried() {
        let client = endpoint().await;
        let gone = endpoint().await;
        let gone_id = gone.id();
        gone.close().await;
        let envelope = sent_from_home(&"ab".repeat(32), "lobby", crate::time::unix_ms());
        assert!(matches!(
            deliver(&client, gone_id, &envelope).await,
            Outcome::Retry(_)
        ));
    }

    #[tokio::test]
    async fn a_partial_frame_times_out_before_the_next_stream_is_answered() {
        let server = endpoint().await;
        let client = endpoint().await;
        let frames = Arc::new(Mutex::new(Vec::new()));
        let router = Router::builder(server.clone())
            .accept(
                ENVELOPE_ALPN,
                DeadlineCaptureFrames {
                    frames: frames.clone(),
                    deadline: Duration::from_millis(100),
                },
            )
            .spawn();
        let conn = connect(&client, loopback_addr(&server), ENVELOPE_ALPN)
            .await
            .unwrap();

        let (mut first_send, mut first_recv) = conn.open_bi().await.unwrap();
        first_send.write_all(&[0, 0]).await.unwrap();

        let (mut second_send, mut second_recv) = conn.open_bi().await.unwrap();
        write_frame(&mut second_send, b"valid second frame")
            .await
            .unwrap();
        second_send.finish().unwrap();

        let second_reply = tokio::time::timeout(Duration::from_secs(2), async {
            let frame = read_frame(&mut second_recv).await.unwrap().unwrap();
            serde_json::from_slice::<Reply>(&frame).unwrap()
        })
        .await
        .expect("second stream should be answered after the first times out");
        assert!(matches!(second_reply, Reply::Applied));
        assert_eq!(
            frames.lock().await.as_slice(),
            &[b"valid second frame".to_vec()]
        );

        let first_reply =
            tokio::time::timeout(Duration::from_secs(1), first_recv.read_to_end(16)).await;
        assert!(
            matches!(first_reply, Ok(Err(_))),
            "first stream should be reset"
        );

        drop(first_send);
        conn.close(0_u32.into(), b"done");
        client.close().await;
        server.close().await;
        router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn flush_delivers_due_envelopes_and_records_each_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let server = endpoint().await;
        let (_router, addr, bo) = lobby(&server).await;
        let lookup = MemoryLookup::new();
        lookup.add_endpoint_info(addr.clone());
        let client = endpoint_with(&lookup).await;
        let at = crate::time::unix_ms();

        let mut queue = Outbox::open(dir.path().join("outbox.jsonl")).unwrap();
        let target = addr.id.to_string();
        queue
            .enqueue(&target, sent_from_home(&bo, "lobby", at))
            .unwrap();
        queue
            .enqueue(&target, sent_from_home(&bo, "elsewhere", at))
            .unwrap();
        let outbox = Mutex::new(queue);

        assert_eq!(flush(&outbox, &client, NOW).await.unwrap(), 2);
        let queue = outbox.lock().await;
        let delivered = queue
            .entries()
            .filter(|e| e.finish == Some(Finish::Delivered))
            .count();
        let refused = queue
            .entries()
            .filter(|e| e.finish == Some(Finish::Refused))
            .count();
        assert_eq!((delivered, refused), (1, 1));
    }

    /// A delivery to an address nothing answers must give up at its deadline.
    #[tokio::test]
    async fn a_delivery_to_an_unanswered_address_gives_up_at_its_deadline() {
        let client = endpoint().await;
        let started = Instant::now();
        let outcome = send_within(
            &client,
            ENVELOPE_ALPN,
            unanswered_addr(),
            b"{}",
            Duration::from_millis(500),
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(outcome, Outcome::Retry("delivery timed out".to_owned()));
    }

    /// A delivery by node ID whose address lookup never answers gives up at its
    /// deadline. The lookup is inside the deadline, so this is the step that hung.
    #[tokio::test]
    async fn a_delivery_whose_lookup_never_answers_gives_up_at_its_deadline() {
        let client = endpoint_with_silent_lookup().await;
        let started = Instant::now();
        let outcome = send_within(
            &client,
            ENVELOPE_ALPN,
            unanswered_id(),
            b"{}",
            Duration::from_millis(500),
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(outcome, Outcome::Retry("delivery timed out".to_owned()));
    }
}
