//! Spike S4 for the id design: payload storage with iroh-blobs, and authorised fetch.
//!
//! (a) p2panda body hash vs iroh-blobs hash for the same bytes.
//! (b) fetch by hash with no ticket over presets::Minimal on localhost.
//! (c) EventMask Intercept gating of get and get_many on the wire, with an allow list.
//! (d) verified streaming: corrupted bytes, on disk and on the wire, are rejected.
//! (e) MemStore vs FsStore: timing, inlining, persistence and file sizes.

use std::{
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey,
    address_lookup::{EndpointInfo, MemoryLookup},
    endpoint::{Connection, VarInt, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_blobs::{
    ALPN, BlobsProtocol, Hash as BlobHash,
    api::{Store, downloader::Downloader},
    protocol::{ChunkRanges, ERR_INTERNAL, ERR_PERMISSION, GetManyRequest, GetRequest, Request},
    provider::events::{
        AbortReason, ConnectMode, EventMask, EventSender, ProviderMessage, RequestMode,
    },
    store::{
        fs::{
            FsStore,
            options::{InlineOptions, Options as FsOptions},
        },
        mem::MemStore,
    },
};
use p2panda_core::{Body, Hash as PandaHash};

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;
/// iroh-blobs verifies in 16 KiB blocks: chunk log 4 over 1 KiB BLAKE3 chunks.
const BLOCK: usize = 16 * KIB;

/// Deterministic pseudo-random bytes, so no payload is trivially compressible.
fn payload(seed: &str, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    blake3::Hasher::new_derive_key(seed)
        .finalize_xof()
        .fill(&mut out);
    out
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// `presets::Minimal` has no relay, so nothing reports a direct address. Wait for the local one.
async fn ready_addr(ep: &Endpoint) -> EndpointAddr {
    for _ in 0..100 {
        let addr = ep.addr();
        if !addr.addrs.is_empty() {
            return addr;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    ep.addr()
}

/// Serve `store` on a `presets::Minimal` endpoint: no relay, no address lookup.
async fn serve(store: &Store, events: Option<EventSender>) -> Result<(Router, EndpointAddr)> {
    let ep = Endpoint::bind(presets::Minimal).await?;
    let addr = ready_addr(&ep).await;
    let blobs = BlobsProtocol::new(store, events);
    let router = Router::builder(ep).accept(ALPN, blobs).spawn();
    Ok((router, addr))
}

/// A `presets::Minimal` requester. The Downloader takes provider IDs, so it needs a lookup that
/// maps them to addresses. A raw `connect` by full address needs no lookup.
async fn requester(secret: Option<SecretKey>, lookup: Option<MemoryLookup>) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::Minimal);
    if let Some(key) = secret {
        builder = builder.secret_key(key);
    }
    if let Some(lookup) = lookup {
        builder = builder.address_lookup(lookup);
    }
    Ok(builder.bind().await?)
}

// ---------------------------------------------------------------------------------------------
// (a) hash equivalence
// ---------------------------------------------------------------------------------------------

async fn part_a() -> Result<()> {
    println!("== (a) hash equivalence: p2panda Body::hash() vs iroh-blobs hash");
    let store = MemStore::new();
    for size in [0, KIB, 64 * KIB, MIB, 10 * MIB] {
        let bytes = payload("s4 hash", size);
        let body = Body::from_bytes(&bytes).hash();
        let digest = PandaHash::digest(&bytes);
        let blob = store.add_slice(&bytes).await?.hash;
        let equal = body == digest
            && body.as_bytes() == blob.as_bytes()
            && BlobHash::new(&bytes) == blob
            && blake3::hash(&bytes).as_bytes() == blob.as_bytes();
        println!(
            "size={size:>9} equal={equal:<5} body={} blob={}",
            hex::encode(body.as_bytes()),
            hex::encode(blob.as_bytes())
        );
        ensure!(equal, "hash mismatch at {size} bytes");
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// (b) fetch by hash, no ticket
// ---------------------------------------------------------------------------------------------

async fn part_b() -> Result<()> {
    println!("\n== (b) fetch by hash, no ticket, presets::Minimal on localhost");
    let provider_store = MemStore::new();
    let bytes = payload("s4 fetch", 10 * MIB);
    let hash = provider_store.add_slice(&bytes).await?.hash;
    let (router, provider) = serve(&provider_store, None).await?;
    println!(
        "provider id={} direct_addrs={}",
        provider.id.fmt_short(),
        provider.addrs.len()
    );

    for run in 1..=3 {
        // The requester has only the provider's address (from addr()) and the hash. No BlobTicket.
        let lookup = MemoryLookup::new();
        lookup.add_endpoint_info(EndpointInfo::from(provider.clone()));
        let ep = requester(None, Some(lookup)).await?;
        let store = MemStore::new();
        let downloader = Downloader::new(&store, &ep);
        let t0 = Instant::now();
        tokio::time::timeout(
            Duration::from_secs(60),
            downloader.download(hash, vec![provider.id]),
        )
        .await??;
        let took = t0.elapsed();
        let got = store.get_bytes(hash).await?;
        let body = Body::from_bytes(&got).hash();
        let bytes_ok = got[..] == bytes[..];
        let hash_ok = body.as_bytes() == hash.as_bytes();
        println!(
            "Downloader run {run}: bytes_ok={bytes_ok} body_hash_eq_requested={hash_ok} took_ms={:.1}",
            ms(took)
        );
        ensure!(bytes_ok && hash_ok, "downloader run {run} failed");
        ep.close().await;
    }

    // Same fetch with no lookup at all: connect by address, then a bare GetRequest for the hash.
    let ep = requester(None, None).await?;
    let store = MemStore::new();
    let t0 = Instant::now();
    let conn = ep.connect(provider.clone(), ALPN).await?;
    store
        .remote()
        .execute_get(conn, GetRequest::blob(hash))
        .complete()
        .await?;
    let took = t0.elapsed();
    let got = store.get_bytes(hash).await?;
    let ok = got[..] == bytes[..];
    println!(
        "no lookup, connect(addr) + GetRequest::blob(hash): bytes_ok={ok} took_ms={:.1}",
        ms(took)
    );
    ensure!(ok, "no-lookup fetch failed");
    ep.close().await;
    router.shutdown().await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// (c) authorised fetch
// ---------------------------------------------------------------------------------------------

fn decide(allow: &HashSet<EndpointId>, who: Option<EndpointId>) -> Result<(), AbortReason> {
    match who {
        Some(id) if allow.contains(&id) => Ok(()),
        _ => Err(AbortReason::Permission),
    }
}

fn label(who: Option<EndpointId>) -> String {
    who.map_or("unknown".to_string(), |id| id.fmt_short().to_string())
}

/// Allow-list gate for get and get_many. `connected` is Notify only so the handler can map
/// connection_id to EndpointId: GetRequestReceived carries the connection ID, not the peer.
/// Fail-closed: a request on an unknown connection is refused.
fn spawn_gate(
    allow: HashSet<EndpointId>,
    mut rx: tokio::sync::mpsc::Receiver<ProviderMessage>,
) -> Arc<Mutex<Vec<String>>> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&log);
    tokio::spawn(async move {
        let mut peers: HashMap<u64, EndpointId> = HashMap::new();
        while let Some(msg) = rx.recv().await {
            match msg {
                ProviderMessage::ClientConnectedNotify(m) => {
                    if let Some(id) = m.inner.endpoint_id {
                        peers.insert(m.inner.connection_id, id);
                    }
                }
                ProviderMessage::ConnectionClosed(m) => {
                    peers.remove(&m.inner.connection_id);
                }
                ProviderMessage::GetRequestReceived(m) => {
                    let who = peers.get(&m.inner.connection_id).cloned();
                    let verdict = decide(&allow, who.clone());
                    sink.lock().unwrap().push(format!(
                        "get {} from {} -> {verdict:?}",
                        m.inner.request.hash,
                        label(who)
                    ));
                    m.tx.send(verdict).await.ok();
                }
                ProviderMessage::GetManyRequestReceived(m) => {
                    let who = peers.get(&m.inner.connection_id).cloned();
                    let verdict = decide(&allow, who.clone());
                    sink.lock().unwrap().push(format!(
                        "get_many of {} hashes from {} -> {verdict:?}",
                        m.inner.request.hashes.len(),
                        label(who)
                    ));
                    m.tx.send(verdict).await.ok();
                }
                _ => {}
            }
        }
    });
    log
}

/// One request on the wire: a GetRequest for one hash, or a GetManyRequest for several.
/// On refusal, returns the message and the stream reset code the provider sent.
async fn wire_fetch(
    ep: &Endpoint,
    provider: &EndpointAddr,
    hashes: &[BlobHash],
    store: &MemStore,
) -> Result<std::result::Result<(), (String, Option<VarInt>)>> {
    let conn = ep.connect(provider.clone(), ALPN).await?;
    let res = if let [one] = hashes {
        store
            .remote()
            .execute_get(conn, GetRequest::blob(*one))
            .complete()
            .await
    } else {
        let req: GetManyRequest = hashes.iter().copied().collect();
        store.remote().execute_get_many(conn, req).complete().await
    };
    Ok(res
        .map(|_| ())
        .map_err(|e| (format!("{e:#}"), e.iroh_error_code())))
}

async fn part_c() -> Result<()> {
    println!("\n== (c) authorised fetch: EventMask get and get_many = RequestMode::Intercept");
    let big = payload("s4 gate big", MIB);
    let small = payload("s4 gate small", 64 * KIB);

    let gated_store = MemStore::new();
    let h_big = gated_store.add_slice(&big).await?.hash;
    let h_small = gated_store.add_slice(&small).await?.hash;
    let open_store = MemStore::new();
    open_store.add_slice(&big).await?;
    open_store.add_slice(&small).await?;

    let allowed_key = SecretKey::generate();
    let allowed_id = allowed_key.public();
    let denied_key = SecretKey::generate();

    // get and get_many: Intercept. connected: Notify, so the handler learns each peer's ID.
    // Everything else stays at EventMask::DEFAULT (push disabled, observe none, no throttle).
    let mask = EventMask {
        connected: ConnectMode::Notify,
        get: RequestMode::Intercept,
        get_many: RequestMode::Intercept,
        ..EventMask::DEFAULT
    };
    let (events, rx) = EventSender::channel(64, mask);
    let gate_log = spawn_gate(HashSet::from([allowed_id]), rx);
    let (gated_router, gated) = serve(&gated_store, Some(events)).await?;
    // No event sender: the default mask, so no intercept.
    let (open_router, open) = serve(&open_store, None).await?;

    let lookup = MemoryLookup::new();
    lookup.add_endpoint_info(EndpointInfo::from(gated.clone()));
    lookup.add_endpoint_info(EndpointInfo::from(open.clone()));
    let allowed = requester(Some(allowed_key), Some(lookup.clone())).await?;
    let denied = requester(Some(denied_key), Some(lookup)).await?;
    ensure!(allowed.id() == allowed_id, "allowed key mismatch");
    println!(
        "allow list = {{A}}; A={} D={} gated={} open={}",
        allowed.id().fmt_short(),
        denied.id().fmt_short(),
        gated.id.fmt_short(),
        open.id.fmt_short()
    );

    let both = vec![h_big, h_small];
    let original = |h: &BlobHash| if *h == h_big { &big } else { &small };
    let rows: Vec<(&str, &Endpoint, &EndpointAddr, Vec<BlobHash>, bool)> = vec![
        ("A -> gated, get", &allowed, &gated, vec![h_big], true),
        ("D -> gated, get", &denied, &gated, vec![h_big], false),
        ("A -> gated, get_many", &allowed, &gated, both.clone(), true),
        ("D -> gated, get_many", &denied, &gated, both.clone(), false),
        ("D -> open, get", &denied, &open, vec![h_big], true),
        ("D -> open, get_many", &denied, &open, both.clone(), true),
    ];

    let mut all_as_expected = true;
    for (name, ep, provider, hashes, expect_served) in rows {
        let store = MemStore::new();
        let (served, reset, detail) = match wire_fetch(ep, provider, &hashes, &store).await? {
            Ok(()) => {
                let mut ok = true;
                for h in &hashes {
                    ok &= store.get_bytes(*h).await?[..] == original(h)[..];
                }
                (true, None, format!("bytes_ok={ok}"))
            }
            Err((msg, code)) => (false, code, msg),
        };
        let as_expected =
            served == expect_served && (expect_served || reset == Some(ERR_PERMISSION));
        all_as_expected &= as_expected;
        println!(
            "{name:<22} served={served:<5} reset={:<10} {detail} => {}",
            format!("{reset:?}"),
            if as_expected {
                "as expected"
            } else {
                "UNEXPECTED"
            }
        );
    }

    // The Downloader path for a single get: the same gate applies.
    let store = MemStore::new();
    let dl_allowed = Downloader::new(&store, &allowed);
    let a_ok = tokio::time::timeout(
        Duration::from_secs(60),
        dl_allowed.download(h_big, vec![gated.id]),
    )
    .await?
    .is_ok();
    let denied_store = MemStore::new();
    let dl_denied = Downloader::new(&denied_store, &denied);
    let d_ok = tokio::time::timeout(
        Duration::from_secs(60),
        dl_denied.download(h_big, vec![gated.id]),
    )
    .await?
    .is_ok();
    println!("Downloader get: A -> gated served={a_ok}; D -> gated served={d_ok}");
    all_as_expected &= a_ok && !d_ok;

    println!("gate decisions (provider side):");
    for line in gate_log.lock().unwrap().iter() {
        println!("    {line}");
    }

    allowed.close().await;
    denied.close().await;
    gated_router.shutdown().await?;
    open_router.shutdown().await?;
    ensure!(all_as_expected, "a gating case was not as expected");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// (d) verified streaming
// ---------------------------------------------------------------------------------------------

/// Flip one bit at `offset` in a file the store owns.
fn flip_bit(path: &Path, offset: u64) -> Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut byte = [0u8; 1];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut byte)?;
    byte[0] ^= 0x01;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&byte)?;
    file.sync_all()?;
    Ok(())
}

/// FsStore at `root`. With `no_inline`, every outboard is a file, not a row in the database.
async fn open_fs(root: &Path, no_inline: bool) -> Result<FsStore> {
    let mut opts = FsOptions::new(root);
    if no_inline {
        opts.inline = InlineOptions::NO_INLINE;
    }
    Ok(FsStore::load_with_opts(root.join("blobs.db"), opts).await?)
}

/// Store `bytes` in a new FsStore at `root`, then shut it down so its files can be edited.
async fn write_fs_blob(root: &Path, bytes: &[u8], no_inline: bool) -> Result<BlobHash> {
    let store = open_fs(root, no_inline).await?;
    let hash = store.add_slice(bytes).await?.hash;
    store.wait_idle().await?;
    store.shutdown().await?;
    Ok(hash)
}

/// Serve the FsStore at `root` and fetch `hash` into a fresh MemStore with a bare GetRequest.
async fn fetch_from_fs(
    root: &Path,
    hash: BlobHash,
    no_inline: bool,
) -> Result<(MemStore, Result<(), String>, Option<VarInt>)> {
    let store = open_fs(root, no_inline).await?;
    let (router, provider) = serve(&store, None).await?;
    let ep = requester(None, None).await?;
    let conn = ep.connect(provider, ALPN).await?;
    let fetched = MemStore::new();
    let res = fetched
        .remote()
        .execute_get(conn, GetRequest::blob(hash))
        .complete()
        .await;
    let (outcome, code) = match res {
        Ok(_) => (Ok(()), None),
        Err(e) => (Err(format!("{e:#}")), e.iroh_error_code()),
    };
    ep.close().await;
    // BlobsProtocol::shutdown also shuts the store down.
    router.shutdown().await?;
    Ok((fetched, outcome, code))
}

/// Walk the requester's store one 16 KiB block at a time. `export_ranges` fails for any byte range
/// the store does not hold, so a block counts as held only if its bytes export. Returns the number
/// of held blocks, whether they form one prefix from block 0, whether every held block matches the
/// original bytes, and whether `bad_block` is held.
async fn scan_blocks(
    fetched: &MemStore,
    hash: BlobHash,
    bytes: &[u8],
    bad_block: usize,
) -> (usize, bool, bool, bool) {
    let (mut held, mut prefix, mut correct, mut bad_held) = (0usize, true, true, false);
    for b in 0..bytes.len() / BLOCK {
        let range = (b * BLOCK) as u64..((b + 1) * BLOCK) as u64;
        // With no size yet, the store clips the range to zero bytes and returns Ok with no data.
        // So a block counts as held only when a full block comes back.
        if let Ok(data) = fetched
            .blobs()
            .export_ranges(hash, range)
            .concatenate()
            .await
        {
            if data.len() == BLOCK {
                correct &= data[..] == bytes[b * BLOCK..(b + 1) * BLOCK];
                prefix &= b == held;
                bad_held |= b == bad_block;
                held += 1;
            }
        }
    }
    (held, prefix, correct, bad_held)
}

/// The provider holds corrupted bytes on disk. Expect: the provider cannot export them, the
/// requester gets a stream reset and keeps nothing.
async fn provider_refuses(
    label: &str,
    root: &Path,
    hash: BlobHash,
    bytes: &[u8],
    bad_block: usize,
    no_inline: bool,
) -> Result<bool> {
    let local = open_fs(root, no_inline).await?;
    let local_export = local
        .export_bao(hash, ChunkRanges::all())
        .bao_to_vec()
        .await;
    println!(
        "{label}: provider local export ok={} {}",
        local_export.is_ok(),
        local_export
            .as_ref()
            .err()
            .map(|e| format!("({e:#})"))
            .unwrap_or_default()
    );
    local.shutdown().await?;

    let (fetched, outcome, code) = fetch_from_fs(root, hash, no_inline).await?;
    println!(
        "{label}: fetch_error={:?} reset_code={code:?}",
        outcome.as_ref().err()
    );
    let (held, prefix, correct, bad_held) = scan_blocks(&fetched, hash, bytes, bad_block).await;
    println!(
        "{label}: held_blocks={held} contiguous_from_0={prefix} held_correct={correct} bad_block_held={bad_held}"
    );
    Ok(outcome.is_err()
        && code == Some(ERR_INTERNAL)
        && local_export.is_err()
        && prefix
        && correct
        && !bad_held)
}

/// Test provider for the wire case: serves a blob's bao encoding with one bit flipped in its final
/// byte. The final byte is in the last leaf, so only that block should fail verification.
#[derive(Debug, Clone)]
struct CorruptingProvider {
    store: Store,
}

impl ProtocolHandler for CorruptingProvider {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = self.serve(&conn).await {
            println!("    corrupting provider: {e:#}");
        }
        Ok(())
    }
}

impl CorruptingProvider {
    async fn serve(&self, conn: &Connection) -> Result<()> {
        let (mut send, mut recv) = conn.accept_bi().await?;
        let (request, _) = Request::read_async(&mut recv).await?;
        let Request::Get(get) = request else {
            return Ok(());
        };
        let mut bytes = self
            .store
            .export_bao(get.hash, ChunkRanges::all())
            .bao_to_vec()
            .await?;
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        send.write_all(&bytes).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }
}

/// One bit flipped on the wire, in the last leaf. Expect: the requester rejects that block and
/// keeps the verified blocks before it.
async fn wire_flip(bytes: &[u8]) -> Result<bool> {
    let wire_store = MemStore::new();
    let hash = wire_store.add_slice(bytes).await?.hash;
    let ep = Endpoint::bind(presets::Minimal).await?;
    let addr = ready_addr(&ep).await;
    let provider = CorruptingProvider {
        store: wire_store.as_ref().clone(),
    };
    let router = Router::builder(ep).accept(ALPN, provider).spawn();

    let req_ep = requester(None, None).await?;
    let conn = req_ep.connect(addr, ALPN).await?;
    let fetched = MemStore::new();
    let res = fetched
        .remote()
        .execute_get(conn, GetRequest::blob(hash))
        .complete()
        .await;
    let outcome = res.as_ref().map(|_| ()).map_err(|e| format!("{e:#}"));
    let code = res.as_ref().err().and_then(|e| e.iroh_error_code());
    println!(
        "wire flip, last leaf: fetch_error={:?} reset_code={code:?}",
        outcome.as_ref().err()
    );
    let last_block = bytes.len() / BLOCK - 1;
    let (held, prefix, correct, bad_held) = scan_blocks(&fetched, hash, bytes, last_block).await;
    println!(
        "wire flip, last leaf: held_blocks={held} contiguous_from_0={prefix} held_correct={correct} bad_block_held={bad_held}"
    );
    let hash_error = matches!(&outcome, Err(msg) if msg.contains("hash mismatch"));

    req_ep.close().await;
    router.shutdown().await?;
    Ok(hash_error && code.is_none() && held == last_block && prefix && correct && !bad_held)
}

async fn part_d() -> Result<()> {
    println!("\n== (d) verified streaming: corrupted bytes are rejected");
    let dir = tempfile::tempdir()?;
    let bytes = payload("s4 verify", MIB); // 64 blocks of 16 KiB

    // Control: an untouched FsStore serves every byte.
    let control = dir.path().join("control");
    let hash = write_fs_blob(&control, &bytes, false).await?;
    let (fetched, outcome, _) = fetch_from_fs(&control, hash, false).await?;
    let ok = outcome.is_ok() && fetched.get_bytes(hash).await?[..] == bytes[..];
    println!("control (on disk, untouched): served_ok={ok}");
    ensure!(ok, "control fetch failed");

    // Case 1: one bit flipped in the stored data file, inside block 37.
    let data_root = dir.path().join("data-flip");
    let hash = write_fs_blob(&data_root, &bytes, false).await?;
    let data_file = data_root
        .join("data")
        .join(format!("{}.data", hash.to_hex()));
    flip_bit(&data_file, (37 * BLOCK + 123) as u64)?;
    let case1 = provider_refuses(
        "case 1 data bit flip, block 37",
        &data_root,
        hash,
        &bytes,
        37,
        false,
    )
    .await?;

    // Case 2: one bit flipped in the stored outboard, in the pair for blocks 32..63.
    // Layout: 64-byte (left, right) hash pairs in pre-order, no size header (the size is kept in
    // the database). Pre-order index 32 is the pair for blocks 32..63 (64 blocks, so 63 pairs).
    let obao_root = dir.path().join("obao-flip");
    let hash = write_fs_blob(&obao_root, &bytes, true).await?;
    let obao_file = obao_root
        .join("data")
        .join(format!("{}.obao4", hash.to_hex()));
    println!(
        "case 2 outboard file len={} (expected 63 * 64 = 4032)",
        std::fs::metadata(&obao_file)?.len()
    );
    flip_bit(&obao_file, (32 * 64 + 5) as u64)?;
    let case2 = provider_refuses(
        "case 2 outboard bit flip",
        &obao_root,
        hash,
        &bytes,
        32,
        true,
    )
    .await?;

    // Case 3: the provider's bytes are fine, one bit is flipped on the wire.
    let case3 = wire_flip(&bytes).await?;

    println!(
        "cases: data_flip_rejected={case1} outboard_flip_rejected={case2} wire_flip_rejected={case3}"
    );
    ensure!(
        case1 && case2 && case3,
        "a corrupted block was not rejected as expected"
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// (e) storage notes
// ---------------------------------------------------------------------------------------------

fn list_files(root: &Path) -> Result<Vec<(String, u64)>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root)?.display().to_string();
                out.push((rel, entry.metadata()?.len()));
            }
        }
    }
    out.sort();
    Ok(out)
}

async fn part_e() -> Result<()> {
    println!("\n== (e) storage: MemStore vs FsStore");
    let bytes = payload("s4 store", 10 * MIB);
    for run in 1..=3 {
        let mem = MemStore::new();
        let t = Instant::now();
        let hash = mem.add_slice(&bytes).await?.hash;
        let add = t.elapsed();
        let t = Instant::now();
        mem.get_bytes(hash).await?;
        let read = t.elapsed();
        println!(
            "MemStore run {run}: add_10MiB_ms={:.1} read_10MiB_ms={:.1}",
            ms(add),
            ms(read)
        );
    }

    let dir = tempfile::tempdir()?;
    for run in 1..=3 {
        let root = dir.path().join(format!("fs{run}"));
        let fs = open_fs(&root, false).await?;
        let t = Instant::now();
        let hash = fs.add_slice(&bytes).await?.hash;
        fs.wait_idle().await?;
        let add = t.elapsed();
        let t = Instant::now();
        fs.get_bytes(hash).await?;
        let read = t.elapsed();
        println!(
            "FsStore run {run}: add_10MiB_ms={:.1} read_10MiB_ms={:.1}",
            ms(add),
            ms(read)
        );
        fs.shutdown().await?;
    }

    println!("-- FsStore layout by size (default inline limits: data and outboard up to 16 KiB)");
    for size in [KIB, 16 * KIB, 17 * KIB, 64 * KIB, MIB, 10 * MIB] {
        let root = dir.path().join(format!("layout-{size}"));
        let fs = open_fs(&root, false).await?;
        let hash = fs.add_slice(&payload("s4 layout", size)).await?.hash;
        fs.wait_idle().await?;
        fs.shutdown().await?;
        let fs = open_fs(&root, false).await?;
        let persisted = fs.has(hash).await?;
        println!("size={size:>9} persisted_after_reopen={persisted}");
        for (name, len) in list_files(&root)? {
            println!("    {name} {len}");
        }
        fs.shutdown().await?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    part_a().await?;
    part_b().await?;
    part_c().await?;
    part_d().await?;
    part_e().await?;
    println!("\nALL PARTS PASSED");
    Ok(())
}
