use anyhow::Result;
use distributed_topic_tracker::{
    AutoDiscoveryGossip, RecordPublisher, RotationHandle, SecretRotation, TopicId,
};
use ed25519_dalek::SigningKey;
use iroh::{Endpoint, SecretKey};
use iroh_gossip::{api::Event, net::Gossip};

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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_thread_ids(true)
        .with_ansi(true)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "distributed_topic_tracker=info".parse().unwrap()),
        )
        .init();

    let secret_key = SecretKey::generate(&mut rand::rng());
    let signing_key = SigningKey::from_bytes(&secret_key.to_bytes());

    let endpoint = Endpoint::builder()
        .secret_key(secret_key.clone())
        .bind()
        .await?;

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let _router = iroh::protocol::Router::builder(endpoint.clone())
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

    println!("joined topic: {}", topic_id.raw());
    let node_id = endpoint.id().to_string();
    println!("node id: {}", &node_id[..8]);

    tokio::spawn(async move {
        while let Some(event) = gossip_receiver.next().await {
            match event {
                Ok(Event::Received(msg)) => {
                    println!(
                        "\n{}: {}",
                        &msg.delivered_from.to_string()[..8],
                        String::from_utf8_lossy(&msg.content)
                    );
                }
                Ok(Event::NeighborUp(peer)) => {
                    println!("\njoined: {}", &peer.to_string()[..8]);
                }
                Ok(Event::NeighborDown(peer)) => {
                    println!("\nleft: {}", &peer.to_string()[..8]);
                }
                _ => {}
            }
        }
    });

    let stdin = std::io::stdin();
    let mut buffer = String::new();
    loop {
        print!("\n> ");
        stdin.read_line(&mut buffer)?;
        let msg = buffer.trim_end().to_string();
        if !msg.is_empty() {
            gossip_sender.broadcast(msg.into()).await?;
            println!(" - (sent)");
        }
        buffer.clear();
    }
}
