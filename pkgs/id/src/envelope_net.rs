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

use crate::directory::{Envelope, Received, Refusal};
use crate::directory_auth::Caller;
use crate::directory_view::{DirectoryAction, DirectoryOutcome};
use crate::envelope_outbox::Outcome;
use crate::world_hub::WorldHub;
use crate::world_net::{read_frame, write_frame};

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
pub async fn send(endpoint: &Endpoint, alpn: &[u8], target: EndpointAddr, json: &[u8]) -> Outcome {
    match tokio::time::timeout(TIMEOUT, exchange(endpoint, alpn, target, json)).await {
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
    let conn = endpoint
        .connect(target, alpn)
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

/// Answers every frame on `conn` with the [`Reply`] `receive` gives it, until the peer stops.
pub async fn answer_frames<F, Fut>(conn: &Connection, receive: F) -> Result<(), AcceptError>
where
    F: Fn(Vec<u8>) -> Fut,
    Fut: Future<Output = Reply> + Send,
{
    while let Ok((mut send, mut recv)) = conn.accept_bi().await {
        let reply = match read_frame(&mut recv).await {
            Ok(Some(body)) => receive(body).await,
            Ok(None) => continue,
            Err(error) => Reply::Refused {
                message: format!("{error:#}"),
            },
        };
        let Ok(json) = serde_json::to_vec(&reply) else {
            continue;
        };
        if write_frame(&mut send, &json).await.is_ok() {
            let _ = send.finish();
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
    use iroh::{
        address_lookup::MemoryLookup,
        endpoint::{RelayMode, presets},
        protocol::Router,
    };

    use super::*;
    use crate::directory::Directory;
    use crate::envelope_outbox::{Finish, Outbox, flush};
    use crate::world::{WorldCore, WorldHandle, WorldLimits};
    use crate::world_session::WorldService;
    use tokio::sync::Mutex;

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
        let at = crate::world::unix_ms();
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

    #[tokio::test]
    async fn an_unreachable_recipient_is_retried() {
        let client = endpoint().await;
        let gone = endpoint().await;
        let gone_id = gone.id();
        gone.close().await;
        let envelope = sent_from_home(&"ab".repeat(32), "lobby", crate::world::unix_ms());
        assert!(matches!(
            deliver(&client, gone_id, &envelope).await,
            Outcome::Retry(_)
        ));
    }

    #[tokio::test]
    async fn flush_delivers_due_envelopes_and_records_each_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let server = endpoint().await;
        let (_router, addr, bo) = lobby(&server).await;
        let lookup = MemoryLookup::new();
        lookup.add_endpoint_info(addr.clone());
        let client = endpoint_with(&lookup).await;
        let at = crate::world::unix_ms();

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
}
