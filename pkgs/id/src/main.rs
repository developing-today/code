use std::path::PathBuf;
use iroh::{protocol::Router, Endpoint};
use iroh_blobs::{store::mem::MemStore, ticket::BlobTicket, BlobsProtocol};
use iroh_gossip::{net::Gossip};
use iroh_docs::{protocol::Docs};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let endpoint = Endpoint::bind().await?;
    let store = MemStore::new();
    let blobs = BlobsProtocol::new(&store, None);
    let gossip = Gossip::builder().spawn(endpoint.clone());
    let docs = Docs::memory()
        .spawn(endpoint.clone(), (*blobs).clone(), gossip.clone())
        .await?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    match arg_refs.as_slice() {
        ["send", filename] => {
            let filename: PathBuf = filename.parse()?;
            let abs_path = std::path::absolute(&filename)?;
            let tag = store.blobs().add_path(abs_path).await?;
            let node_id = endpoint.id();
            let ticket = BlobTicket::new(node_id.into(), tag.hash, tag.format);

            println!(
                "<binary> receive {ticket} {}",
                filename.display()
            );

            let router = Router::builder(endpoint)
                .accept(iroh_blobs::ALPN, blobs)
               .accept(iroh_gossip::ALPN, gossip)
               .accept(iroh_docs::ALPN, docs)
               .spawn();
            tokio::signal::ctrl_c().await?;
            router.shutdown().await?;
        }
        ["receive", ticket, filename] => {
            let filename: PathBuf = filename.parse()?;
            let abs_path = std::path::absolute(filename)?;
            let ticket: BlobTicket = ticket.parse()?;
            let downloader = store.downloader(&endpoint);

            println!("Starting download.");

            downloader
                .download(ticket.hash(), Some(ticket.addr().id))
                .await?;

            println!("Finished download.");
            println!("Copying to destination.");

            store.blobs().export(ticket.hash(), abs_path).await?;

            println!("Finished copying.");
            println!("Shutting down.");

            endpoint.close().await;
        }
        _ => {
            println!("Couldn't parse command line arguments: {args:?}");
            println!("Usage:");
            println!("    TODO");
        }
    }
    Ok(())
}
