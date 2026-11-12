//! Facade re-export of the envelope transport (moved to `id-sync` in
//! step 3c of the crate split, #4406). The tests below stay here: they
//! serve envelopes from a real world hub and directory, which live in
//! the facade until steps 5-6.

pub use id_sync::envelope_net::*;

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
    use iroh::{Endpoint, EndpointAddr};

    use crate::directory::{Directory, Envelope};
    use crate::envelope_outbox::Outcome;
    use crate::world_hub::WorldHub;
    use crate::directory_auth::Caller;
    use crate::directory_view::DirectoryAction;
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
}
