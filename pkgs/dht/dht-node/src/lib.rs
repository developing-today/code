//! Endpoint, router, and record-publisher setup for a DHT gossip node.

use anyhow::Result;
use distributed_topic_tracker::{
    AutoDiscoveryGossip, GossipReceiver, GossipSender, RecordPublisher, RotationHandle,
    SecretRotation, TopicId,
};
use ed25519_dalek::SigningKey;
use iroh::protocol::Router;
use iroh::{Endpoint, SecretKey};
use iroh_gossip::net::Gossip;

struct MySecretRotation;

impl SecretRotation for MySecretRotation {
    fn derive(
        &self,
        topic_hash: [u8; 32],
        unix_minute: u64,
        initial_secret_hash: [u8; 32],
    ) -> [u8; 32] {
        dht_rotation::derive(topic_hash, unix_minute, initial_secret_hash)
    }
}

/// The joined topic and its handles. Keep the node alive while using the
/// gossip handles so it retains the endpoint, router, and gossip protocol.
pub struct Node {
    pub topic_id: TopicId,
    pub sender: GossipSender,
    pub receiver: GossipReceiver,
    // Keep the original reverse-local drop order: router, gossip, endpoint.
    _router: Router,
    _gossip: Gossip,
    endpoint: Endpoint,
}

impl Node {
    pub fn node_id(&self) -> String {
        self.endpoint.id().to_string()
    }
}

/// Build a node and join its topic through the existing auto-discovery path.
pub async fn join() -> Result<Node> {
    let secret_key = SecretKey::generate(&mut rand::rng());
    let signing_key = SigningKey::from_bytes(&secret_key.to_bytes());

    let endpoint = Endpoint::builder()
        .secret_key(secret_key.clone())
        .bind()
        .await?;

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let router = Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .spawn();

    let topic_id = TopicId::new("my-iroh-gossip-topic".to_string());
    let initial_secret = b"my-initial-secret".to_vec();

    let record_publisher = RecordPublisher::new(
        topic_id.clone(),
        signing_key.verifying_key(),
        signing_key.clone(),
        Some(RotationHandle::new(MySecretRotation)),
        initial_secret,
    );

    let (gossip_sender, gossip_receiver) = gossip
        .subscribe_and_join_with_auto_discovery(record_publisher)
        .await?
        .split()
        .await?;

    Ok(Node {
        topic_id,
        sender: gossip_sender,
        receiver: gossip_receiver,
        _router: router,
        _gossip: gossip,
        endpoint,
    })
}
