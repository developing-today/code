//! Checks SQL-only semantics of p2panda-store's SQLite implementation (memory store, no test_utils).
use std::collections::BTreeMap;

use futures_util::TryStreamExt;
use p2panda_core::{AnyOperation, Body, Hash, Header, Operation, SeqNum, SigningKey, VerifyingKey};
use p2panda_store::logs::LogStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::{SqliteError, SqliteStore, SqliteStoreBuilder, Transaction};

type Log = u64;

/// Headers with seq > 0 need a backlink to decode (AnyHeader::decode checks the field).
fn op(key: &SigningKey, seq: SeqNum, backlink: Option<Hash>, body: &[u8]) -> Operation<()> {
    let header = Header::<()>::builder()
        .seq_num(seq)
        .backlink(backlink)
        .body(body)
        .build(key, ());
    Operation::from_parts(header, Some(Body::from_bytes(body)))
}

async fn insert(store: &SqliteStore, op: &Operation<()>, log: Log) -> Result<bool, SqliteError> {
    <SqliteStore as OperationStore<Operation<()>, Hash>>::insert_operation(store, &op.hash, op, &log)
        .await
}

/// `insert_operation` goes through `tx()`, so every insert needs an open transaction.
async fn insert_tx(store: &SqliteStore, op: &Operation<()>, log: Log) -> Result<bool, SqliteError> {
    let permit = store.begin().await?;
    let inserted = insert(store, op, log).await?;
    store.commit(permit).await?;
    Ok(inserted)
}

async fn latest(store: &SqliteStore, author: &VerifyingKey, log: Log) -> Result<Option<AnyOperation>, SqliteError> {
    <SqliteStore as LogStore<AnyOperation, VerifyingKey, Log, SeqNum, Hash>>::get_latest_entry(store, author, &log).await
}

async fn heights(store: &SqliteStore, author: &VerifyingKey, logs: &[Log]) -> Result<Option<BTreeMap<Log, SeqNum>>, SqliteError> {
    <SqliteStore as LogStore<AnyOperation, VerifyingKey, Log, SeqNum, Hash>>::get_log_heights(store, author, logs).await
}

async fn size(store: &SqliteStore, author: &VerifyingKey, log: Log, after: Option<SeqNum>, until: Option<SeqNum>) -> Result<Option<(u32, u32)>, SqliteError> {
    <SqliteStore as LogStore<AnyOperation, VerifyingKey, Log, SeqNum, Hash>>::get_log_size(store, author, &log, after, until).await
}

async fn seqs(store: &SqliteStore, author: &VerifyingKey, log: Log, after: Option<SeqNum>, until: Option<SeqNum>) -> Result<Vec<SeqNum>, SqliteError> {
    let stream = <SqliteStore as LogStore<AnyOperation, VerifyingKey, Log, SeqNum, Hash>>::log_entries(store, author, &log, after, until)?;
    let entries: Vec<_> = stream.try_collect().await?;
    Ok(entries.iter().map(|e| e.entry.header.seq_num).collect())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = SqliteStoreBuilder::memory().build().await?;
    let key = SigningKey::from_bytes(&[7; 32]);
    let author = key.verifying_key();

    println!("0 insert without tx: {:?}", insert(&store, &op(&key, 99, None, b"x"), 7).await);

    println!("1 empty latest: {:?}", latest(&store, &author, 0).await?.map(|o| o.header.seq_num));
    println!("1 empty heights: {:?}", heights(&store, &author, &[0]).await?);
    println!("1 empty size: {:?}", size(&store, &author, 0, None, None).await);
    println!("1 empty entries: {:?}", seqs(&store, &author, 0, None, None).await?);

    insert_tx(&store, &op(&key, 9, Some(Hash::digest(b"prev 9")), b"nine"), 1).await?;
    insert_tx(&store, &op(&key, 10, Some(Hash::digest(b"prev 10")), b"ten"), 1).await?;
    println!("2 latest after seq 9 and 10: {:?}", latest(&store, &author, 1).await?.map(|o| o.header.seq_num));

    let a = op(&key, 0, None, b"fork a");
    let b = op(&key, 0, None, b"fork b");
    println!("3 hashes differ: {}", a.hash != b.hash);
    println!("3 insert fork a: {:?}", insert_tx(&store, &a, 2).await);
    println!("3 insert fork b: {:?}", insert_tx(&store, &b, 2).await);
    println!("3 log 2 size (count, bytes): {:?}", size(&store, &author, 2, None, None).await);
    println!("3 log 2 heights: {:?}", heights(&store, &author, &[2]).await?);
    let tie = latest(&store, &author, 2).await?.map(|o| (o.hash == a.hash, o.hash == b.hash));
    println!("3 latest on fork log is (is_a, is_b): {:?}", tie);

    let mut hashes = Vec::new();
    let mut prev: Option<Hash> = None;
    for seq in 0..5 {
        let o = op(&key, seq, prev, format!("m{seq}").as_bytes());
        prev = Some(o.hash);
        hashes.push(o.hash);
        insert_tx(&store, &o, 0).await?;
    }
    println!("4 log 0 all: {:?}", seqs(&store, &author, 0, None, None).await?);
    println!("4 log 0 after=1 until=3: {:?}", seqs(&store, &author, 0, Some(1), Some(3)).await?);
    println!("4 log 0 size after=1 until=3: {:?}", size(&store, &author, 0, Some(1), Some(3)).await);
    println!("4 log 0 heights: {:?}", heights(&store, &author, &[0]).await?);

    let pruned = <SqliteStore as LogStore<AnyOperation, VerifyingKey, Log, SeqNum, Hash>>::prune_entries(&store, &author, &0, &3).await?;
    let has0 = <SqliteStore as OperationStore<Operation<()>, Hash>>::has_operation(&store, &hashes[0]).await?;
    println!("5 prune until 3: deleted {pruned}; has_operation(seq 0) = {has0}");
    println!("5 log 0 after prune: {:?}", seqs(&store, &author, 0, None, None).await?);
    Ok(())
}
