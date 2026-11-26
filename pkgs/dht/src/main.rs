use anyhow::Result;
use iroh_gossip::Event;

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

    let node = dht_node::join().await?;
    let hash = node.topic_id.hash();
    println!(
        "joined topic: {}",
        hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let node_id = node.node_id();
    println!("node id: {}", &node_id[..8]);

    let mut gossip_receiver = node.receiver;
    let gossip_sender = node.sender;
    tokio::spawn(async move {
        while let Ok(event) = gossip_receiver.next().await {
            match event {
                Event::Received(msg) => {
                    println!(
                        "\n{}: {}",
                        &msg.delivered_from.to_string()[..8],
                        String::from_utf8_lossy(&msg.content)
                    );
                }
                Event::NeighborUp(peer) => {
                    println!("\njoined: {}", &peer.to_string()[..8]);
                }
                Event::NeighborDown(peer) => {
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
