use anyhow::result;
use iroh::endpoint::{endpoint, secretkey};
use iroh_gossip::net::gossip;
use distributed_topic_tracker::{autodiscoverygossip, recordpublisher, topicid, secretrotation, rotationhandle};
use ed25519_dalek::signingkey;
use futures_lite::streamext;

struct mysecretrotation;

impl secretrotation for mysecretrotation {
    fn derive(&self, topic_hash: [u8; 32], unix_minute: u64, initial_secret_hash: [u8; 32]) -> [u8; 32] {
        use sha2::{sha512, digest::digest;
        let mut hash = sha512::new();
        hash.update(topic_hash);
        hash.update(unix_minute.to_be_bytes());
        hash.update(initial_secret_hash);
        hash.update(b"rotate");
        hash.finalize()[..32].try_into().expect("hash")
    }
}

#[tokio::main]
async fn main() -> result<()> {
    tracing_subscriber::fmt()
        .with_thread_ids(true)
        .with_ansi(true)
        .with_env_filter(tracing_subscriber::envfilter::try_from_default_env()
            .unwrap_or_else(|_| "distributed_topic_tracker=info".parse().unwrap()))
        .init();

    let secret_key = secretkey::generate(&mut rand::rng());
    let signing_key = signingkey::from_bytes(&secret_key.to_bytes());

    let endpoint = endpoint::builder()
        .secret_key(secret_key.clone())
        .bind().await?;

    let gossip = gossip::builder().spawn(endpoint.clone());
    let _router = iroh::protocol::router::builder(endpoint.clone())
        .accept(iroh_gossip::alpn, gossip.clone())
        .spawn();

    let topic_id = topicid::new("my-iroh-gossip-topic".to_string());
    let initial_secret = b"my-initial-secret".to_vec();

    let record_publisher = recordpublisher::new(
        topic_id.clone(),
        signing_key.verifying_key(),
        signing_key.clone(),
        Some(rotationhandle::new(mysecretrotation)),
        initial_secret,
    );

    let (gossip_sender, mut gossip_receiver) = gossip
        .subscribe_and_join_with_auto_discovery(record_publisher).await?
        .split()?;

    println!("joined topic: {}", &topic_id);
    println!("node id: {}", &endpoint.node_id().to_string()[..8]);

    tokio::spawn(async move {
        while let Some(event) = gossip_receiver.next().await {
            match event {
                Ok(gossip::event::Event::Received(msg)) => {
                    println!("\n{}: {}", &msg.delivered_from.to_string()[..8],
                        String::from_utf8_lossy(&msg.content));
                }
                Ok(gossip::event::Event::NeighborUp(peer)) => {
                    println!("\njoined: {}", &peer.to_string()[..8]);
                }
                Ok(gossip::event::Event::NeighborDown(peer)) => {
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
