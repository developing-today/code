//! Facade re-export of the artifact wire layer (moved to `id-sync` in
//! step 3c of the crate split, #4406). The tests below stay here: they
//! run the protocols against a real `Directory` and world hub, which
//! live in the facade until steps 5-6.

pub use id_sync::artifact::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::net::{Ipv4Addr, SocketAddr};
    use std::time::Instant;

    use iroh::{
        TransportAddr,
        endpoint::{RelayMode, presets},
        protocol::Router,
    };
    use iroh_base::SecretKey;

    use std::time::Duration;

    use anyhow::Result;
    use ed25519_dalek::SigningKey;
    use iroh::{
        Endpoint, EndpointAddr,
        endpoint::Connection,
        protocol::{AcceptError, ProtocolHandler},
    };
    use sha2::{Digest, Sha256};

    use id_core::frames::{read_frame, write_frame};

    use crate::directory::{Directory, Refusal, hex_to_array};
    use crate::directory_auth::Caller;
    use crate::directory_view::DirectoryAction;
    use crate::envelope_outbox::Outcome;
    use crate::world::hex_encode;
    use crate::world_hub::WorldHub;
    use crate::test_forgeries::{IDENTITY_KEY, identity_forgery, with_non_canonical_s};
    use crate::world::{WorldCore, WorldHandle, WorldLimits};
    use crate::world_session::WorldService;
    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }
    fn public(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }
    fn endpoint_of(seed: u8) -> String {
        SecretKey::from_bytes(&[seed; 32]).public().to_string()
    }
    fn document(text: &str) -> Kind {
        Kind::Document(Document {
            media_type: "application/json".to_owned(),
            content: json!({ "text": text }),
        })
    }
    /// Sign a typed kind without the construction-side body check in
    /// [`SignedArtifact::sign_after`], so receipt-side refusal of that
    /// body stays testable (the typed twin of [`SignedArtifact::sign_unchecked`]).
    fn sign_kind_unchecked(key: &SigningKey, kind: &Kind) -> SignedArtifact {
        let (name, body) = kind.encode().unwrap();
        SignedArtifact::sign_unchecked(key, 1, "", name, body).unwrap()
    }
    fn author_tag(subject: &str, author: u8) -> Kind {
        Kind::Tag(Tag {
            subject: subject.to_owned(),
            name: "author".to_owned(),
            value: Some(public(author)),
        })
    }
    fn account(directory: &mut Directory, name: &str) -> (String, String) {
        let (credential, _) = directory.sign_up(name).unwrap();
        let id = directory.account_for(&credential).unwrap();
        (id, credential)
    }
    fn key_of(credential: &str) -> SigningKey {
        let secret = hex_to_array::<32>(credential.rsplit('.').next().unwrap()).unwrap();
        SigningKey::from_bytes(&secret)
    }
    async fn endpoint() -> Endpoint {
        Endpoint::builder(presets::Minimal)
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap()
    }
    fn loopback(endpoint: &Endpoint) -> EndpointAddr {
        EndpointAddr::from_parts(
            endpoint.id(),
            endpoint.bound_sockets().iter().map(|a| {
                let ip = if a.ip().is_unspecified() {
                    Ipv4Addr::LOCALHOST.into()
                } else {
                    a.ip()
                };
                TransportAddr::Ip(SocketAddr::new(ip, a.port()))
            }),
        )
    }
    #[test]
    fn a_page_applies_what_verifies_and_stops_at_a_forgery() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, "home", 2).unwrap();
        home.receive(&accept, "home", 2).unwrap();
        let removal = home.remove_friend(&ann_credential, &bo).unwrap();

        // Unchecked: the crafted declaration's author is the credential
        // key, not the account named in the body, so construction-side
        // validation would refuse it before the tamper under test.
        let mut forged = SignedArtifact::sign_unchecked(
            &key_of(&ann_credential),
            2,
            &removal.hash,
            HOME_DECLARED,
            json!({"account": ann, "servers": []}),
        )
        .unwrap();
        forged.body =
            json!({"account": ann, "servers": [{"endpoint": endpoint_of(9), "world_id": "x"}]});
        let outcome = apply_page(&mut away, &ann, &[removal, forged]);
        assert_eq!(outcome.stored, 1);
        assert_eq!(outcome.refused.map(|refused| refused.seq), Some(2));
        assert!(!away.are_friends(&ann, &bo), "the verified removal applied");
        assert_eq!(away.artifacts().head_seq(&ann), 1);
    }
    #[tokio::test]
    async fn a_pull_over_iroh_applies_the_removal_a_home_server_holds() {
        let mut home = Directory::new();
        let mut away = Directory::new();
        let (ann, ann_credential) = account(&mut home, "Ann");
        let (bo, bo_credential) = account(&mut away, "Bo");
        let (request, _) = home
            .request_friend(&ann_credential, &bo, "away", 1)
            .unwrap();
        away.receive(&request, "away", 1).unwrap();
        let (accept, _) = away.accept_friend(&bo_credential, &ann, "home", 2).unwrap();
        home.receive(&accept, "home", 2).unwrap();
        let removal = home.remove_friend(&ann_credential, &bo).unwrap();

        let world = WorldHandle::spawn(WorldCore::new("home", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, None);
        let stored = service
            .directory(
                Caller::default(),
                DirectoryAction::ReceiveArtifact { artifact: removal },
            )
            .await
            .unwrap();
        assert_eq!(stored.stored, Some(Stored::New));

        let server = endpoint().await;
        let _router = Router::builder(server.clone())
            .accept(
                PULL_ALPN,
                ArtifactPullProtocol::new(WorldHub::single(service)),
            )
            .spawn();
        let client = endpoint().await;
        let outcome = pull(&client, loopback(&server), &ann, &mut away)
            .await
            .unwrap();
        assert_eq!(outcome.stored, 1);
        assert!(outcome.refused.is_none());
        assert!(!away.are_friends(&ann, &bo));
        assert_eq!(away.artifacts().head_seq(&ann), 1);
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn a_silent_or_never_closing_pull_peer_does_not_pin_the_handler() {
        // #2087 regression: accept waited forever for the request
        // frame and for the dialer to close after answering. Both
        // waits are now bounded (short bounds injected here), and
        // the handler future completing is the observable release.
        // No Router here: accept() is driven directly, so the
        // endpoint must advertise the pull ALPN itself.
        let server = Endpoint::builder(presets::Minimal)
            .alpns(vec![PULL_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        let client = endpoint().await;

        // Silent peer: connects, opens a stream, sends nothing.
        let world = WorldHandle::spawn(WorldCore::new("home", WorldLimits::default()).unwrap());
        let protocol = ArtifactPullProtocol {
            hub: WorldHub::single(WorldService::new(world, None)),
            frame_deadline: Duration::from_millis(200),
            close_wait: Duration::from_millis(200),
        };
        let accept_task = tokio::spawn({
            let server = server.clone();
            async move {
                let incoming = server.accept().await.expect("incoming");
                let conn = incoming.await.expect("connecting");
                protocol.accept(conn).await
            }
        });
        let silent = client.connect(loopback(&server), PULL_ALPN).await.unwrap();
        let (mut send, _recv) = silent.open_bi().await.unwrap();
        // Announce the stream with a partial frame-length prefix
        // (an idle stream is not visible to accept_bi), then stall:
        // no request frame ever arrives.
        send.write_all(&[0, 0]).await.unwrap();
        let _send = send;
        let released = tokio::time::timeout(Duration::from_secs(15), accept_task)
            .await
            .expect("a silent peer's handler is released at the frame deadline")
            .expect("the accept task did not panic");
        assert!(
            released.is_ok(),
            "the silent accept ends cleanly: {released:?}"
        );

        // Never-closing peer: reads its answer, then holds the
        // connection open instead of closing it.
        let world = WorldHandle::spawn(WorldCore::new("home", WorldLimits::default()).unwrap());
        let protocol = ArtifactPullProtocol {
            hub: WorldHub::single(WorldService::new(world, None)),
            frame_deadline: Duration::from_secs(30),
            close_wait: Duration::from_millis(200),
        };
        let accept_task = tokio::spawn({
            let server = server.clone();
            async move {
                let incoming = server.accept().await.expect("incoming");
                let conn = incoming.await.expect("connecting");
                protocol.accept(conn).await
            }
        });
        let peer = client.connect(loopback(&server), PULL_ALPN).await.unwrap();
        let (mut send, mut recv) = peer.open_bi().await.unwrap();
        let request = PullRequest {
            account: public(1),
            after: 0,
        };
        write_frame(&mut send, &serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        send.finish().unwrap();
        let answer = read_frame(&mut recv).await.unwrap();
        assert!(
            answer.is_some(),
            "the pull is answered before the close wait"
        );
        let released = tokio::time::timeout(Duration::from_secs(15), accept_task)
            .await
            .expect("a never-closing peer's handler is released at the close wait")
            .expect("the accept task did not panic");
        assert!(
            released.is_ok(),
            "the answered accept ends cleanly: {released:?}"
        );
    }
    #[test]
    fn a_pulled_artifact_from_another_account_is_refused() {
        let mut directory = Directory::new();
        let stranger = SignedArtifact::sign(
            &key(5),
            1,
            "",
            HOME_DECLARED,
            json!({"account": public(5), "servers": []}),
        )
        .unwrap();
        let outcome = apply_page(&mut directory, &public(1), &[stranger]);
        assert_eq!(outcome.stored, 0);
        assert!(outcome.refused.is_some());
    }
    #[test]
    fn documents_and_tags_are_checked_and_never_applied_as_directory_changes() {
        // Construction refuses these bodies itself now (#5623), with the
        // same Forbidden class receipt uses; a raw-signed copy is still
        // refused by parsed() (the symmetry test below pins both sides).
        let huge = Kind::Document(Document {
            media_type: "application/json".to_owned(),
            content: json!({ "text": "x".repeat(MAX_DOCUMENT_BYTES) }),
        });
        let huge_error = ArtifactLog::default().sign(&key(1), &huge).unwrap_err();
        assert!(
            matches!(
                huge_error.downcast_ref::<Refusal>(),
                Some(Refusal::Forbidden(_))
            ),
            "{huge_error:#}"
        );
        assert!(sign_kind_unchecked(&key(1), &huge).parsed().is_err());

        let loose = Kind::Tag(Tag {
            subject: "not a hash".to_owned(),
            name: "author".to_owned(),
            value: None,
        });
        let loose_error = ArtifactLog::default().sign(&key(1), &loose).unwrap_err();
        assert!(
            matches!(
                loose_error.downcast_ref::<Refusal>(),
                Some(Refusal::Forbidden(_))
            ),
            "{loose_error:#}"
        );
        assert!(sign_kind_unchecked(&key(1), &loose).parsed().is_err());

        let fine = ArtifactLog::default()
            .sign(&key(1), &document("ok"))
            .unwrap();
        assert_eq!(fine.parsed().unwrap(), Some(document("ok")));
        let tag = ArtifactLog::default()
            .sign(&key(1), &author_tag(&fine.hash, 1))
            .unwrap();
        assert!(matches!(tag.parsed(), Ok(Some(Kind::Tag(_)))));

        let mut directory = Directory::new();
        let stored = directory
            .receive_artifact(
                &ArtifactLog::default()
                    .sign(&key(1), &document("x"))
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(stored, Stored::New);
        assert_eq!(directory.artifacts().head_seq(&public(1)), 1);
    }
    #[tokio::test]
    async fn a_document_and_its_author_tag_travel_over_iroh() {
        let mut author = Directory::new();
        let (ann, credential) = account(&mut author, "Ann");
        let document = author
            .publish_document(
                &credential,
                "application/json",
                json!({ "title": "Notes", "items": [1, 2] }),
                Vec::new(),
            )
            .unwrap();
        let tag = author
            .publish_tag(
                &credential,
                &document.hash,
                "author",
                Some(ann.clone()),
                Vec::new(),
            )
            .unwrap();
        assert_eq!(tag.seq, 2);

        let world = WorldHandle::spawn(WorldCore::new("home", WorldLimits::default()).unwrap());
        let hub = WorldHub::single(WorldService::new(world, None));
        let server = endpoint().await;
        let _router = Router::builder(server.clone())
            .accept(PUSH_ALPN, ArtifactPushProtocol::new(hub.clone()))
            .accept(PULL_ALPN, ArtifactPullProtocol::new(hub))
            .spawn();
        let client = endpoint().await;
        let home = loopback(&server);

        assert_eq!(
            push(&client, home.clone(), &document).await,
            Outcome::Delivered
        );
        assert_eq!(push(&client, home.clone(), &tag).await, Outcome::Delivered);
        assert_eq!(push(&client, home.clone(), &tag).await, Outcome::Delivered);

        let mut reader = Directory::new();
        let outcome = pull(&client, home, &ann, &mut reader).await.unwrap();
        assert_eq!(outcome.stored, 2);
        let pulled = reader.artifacts().page(&ann, 0);
        assert_eq!(pulled, vec![document.clone(), tag.clone()]);
        match tag.parsed().unwrap() {
            Some(Kind::Tag(read)) => assert_eq!(read.subject, document.hash),
            other => panic!("expected a tag, got {other:?}"),
        }
    }
    #[test]
    fn an_artifact_with_a_non_canonical_s_is_refused() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let mut artifact =
            SignedArtifact::sign(&key, 1, "", "note", serde_json::json!({})).unwrap();
        assert!(artifact.verify().is_ok());
        let raw = hex_to_array::<64>(&artifact.signature).unwrap();
        artifact.signature = hex_encode(&with_non_canonical_s(raw));
        assert!(artifact.verify().is_err());
    }
    #[test]
    fn an_artifact_under_a_small_order_key_is_refused() {
        let author = hex_encode(&IDENTITY_KEY);
        let body = serde_json::json!({});
        let bytes = signed_bytes(&author, 1, "", "note", &body, &[]).unwrap();
        let artifact = SignedArtifact {
            author,
            seq: 1,
            prev: String::new(),
            kind: "note".to_owned(),
            body,
            after: Vec::new(),
            hash: hex_encode(&Sha256::digest(&bytes)),
            signature: hex_encode(&identity_forgery()),
        };
        assert!(artifact.verify().is_err());
    }
    /// A hundred documents at the size limit would be about 6.4 MB in one page,
    /// far over the 1 MiB frame. The pull must take several pages.
    #[tokio::test]
    async fn a_pull_of_maximum_size_documents_takes_several_pages() {
        let mut author = Directory::new();
        let (ann, credential) = account(&mut author, "Ann");
        let world = WorldHandle::spawn(WorldCore::new("home", WorldLimits::default()).unwrap());
        let service = WorldService::new(world, None);
        for _ in 0..100 {
            let document = author
                .publish_document(
                    &credential,
                    "application/json",
                    json!({ "text": "x".repeat(MAX_DOCUMENT_BYTES - 32) }),
                    Vec::new(),
                )
                .unwrap();
            let stored = service
                .directory(
                    Caller::default(),
                    DirectoryAction::ReceiveArtifact { artifact: document },
                )
                .await
                .unwrap();
            assert_eq!(stored.stored, Some(Stored::New));
        }

        let server = endpoint().await;
        let _router = Router::builder(server.clone())
            .accept(
                PULL_ALPN,
                ArtifactPullProtocol::new(WorldHub::single(service)),
            )
            .spawn();
        let client = endpoint().await;
        let mut reader = Directory::new();
        let outcome = pull(&client, loopback(&server), &ann, &mut reader)
            .await
            .unwrap();
        assert_eq!(outcome.stored, 100);
        assert!(outcome.refused.is_none());
        assert_eq!(reader.artifacts().head_seq(&ann), 100);
    }
}
