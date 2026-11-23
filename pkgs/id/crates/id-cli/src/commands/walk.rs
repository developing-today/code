//! The `id walk` commands: the #9106 walking skeleton (#12232)
//! driven by hand — step B3, reaching the path of
//! `id_sync::federation` from the CLI.
//!
//! The path, as the substrate suites run it: a signed record is
//! written at an `id://` address on node A; it is held in A's
//! store; it syncs to node B over the federation exchange
//! protocol; a conflicting record is written on B; the conflict
//! is reported on both nodes; a record that follows both edits
//! resolves it. Here every step is a command against a node
//! directory on disk: the B2(b) store (`DiskReplica`) for the
//! one-shot commands, and the B2(b)+B2(c) composition
//! (`ServingNode`) for the node that serves. Every command opens
//! its node fresh and shuts the store down on the way out, so a
//! node restarts with what it had — the restart is the normal
//! case, not a special mode.
//!
//! One process uses a node directory at a time — the store's
//! rule, surfaced rather than hidden: opening a directory a
//! `walk serve` holds times out with that explanation, instead of
//! waiting on the store's lock forever.
//!
//! The demo world is deliberately small, and says so: one graph
//! (the note the substrate suites write), one scope, and two
//! writers — alice and bob — whose keys are *demonstration keys*,
//! derived deterministically from the writer's name below. Anyone
//! can recompute them; they exist so two terminals can play both
//! writers without a key exchange. Production key management is
//! `id key` / `id keyset`, off this path. Logical time is the
//! wall clock in seconds, so successive commands order the way
//! they were run.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use ed25519_dalek::SigningKey;
use iroh::{
    Endpoint, EndpointAddr, SecretKey, TransportAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use serde::{Deserialize, Serialize};

use id_core::graph::record::{GraphRecord, OpenedRecord, open};
use id_core::graph::term::Term;
use id_core::hash_profile::ProfileDispatch;
use id_core::id_uri_parse::{IdUriV1, UriTarget};
use id_core::signed_envelope::{Envelope as RecordEnvelope, LogicalGraphId};
use id_sync::federation::exchange_net::EXCHANGE_ALPN;
use id_sync::federation::record_store::{DiskReplica, FsBlobs, ServingNode};
use id_sync::federation::slices::b07_conflict_report::{
    ConflictStore, IngestOutcome, ReportId, ReportStatus, RetentionPolicy, VersionHeader,
    conflict_key,
};
use id_sync::federation::slices::b08_sync_scope::SyncScopeConfig;
use id_sync::federation::slices::b09_anti_entropy::{Exchange, Replica};
use id_sync::federation::slices::b32_ingest_gate::{
    Envelope, EventId, FreshnessWindow, LogicalTime, SignerAuthorization, sign_envelope,
};

use crate::cli::WalkCommand;

/// The demo writers. Both are authorized on every walk node: the
/// demo's whole world is these two writers and one graph.
const WRITERS: [&str; 2] = ["alice", "bob"];
/// The demo link's only scope (as in the substrate suites).
const SCOPE: &str = "notes";
/// The demo link's only trust domain.
const TRUST_DOMAIN: &str = "home";
/// The address namespace the demo records are minted under.
const NAMESPACE: &str = "notes";
/// The signing context of the demo records.
const CONTEXT: &str = "namespace/notes";
/// The demo graph: the one object every walk record versions.
const GRAPH: [u8; 32] = [0x11; 32];

/// The node directory's walk configuration file.
const CONFIG_FILE: &str = "walk-node.json";
/// The node directory's endpoint key file (32 raw bytes): the
/// serving endpoint's identity is stable across restarts, so a
/// peer token printed yesterday still names this node.
const ENDPOINT_KEY_FILE: &str = "endpoint.key";

/// A walk node's recorded configuration.
#[derive(Debug, Serialize, Deserialize)]
struct NodeConfig {
    /// The node's name.
    name: String,
}

/// The demonstration key for a demo writer: BLAKE3 of a domain
/// string and the writer's name. Deterministic on purpose — see
/// the module docs. Not a production key.
fn writer_key(writer: &str) -> SigningKey {
    let digest = blake3::hash(format!("id walk demo writer\0{writer}").as_bytes());
    SigningKey::from_bytes(digest.as_bytes())
}

/// A fresh replica for a walk node, with the substrate suites'
/// policy: both demo writers authorized for the demo scope, the
/// same freshness window and retention.
fn replica(name: &str) -> Replica {
    let signers = WRITERS
        .iter()
        .map(|identity| SignerAuthorization {
            identity: (*identity).to_owned(),
            verifying_key: writer_key(identity).verifying_key().to_bytes(),
            scopes: [SCOPE.to_owned()].into_iter().collect(),
        })
        .collect();
    let freshness = FreshnessWindow {
        max_age: 1_000_000,
        max_future_skew: 30,
    };
    Replica::new(
        name,
        TRUST_DOMAIN,
        ConflictStore::new(signers, freshness, RetentionPolicy { retain_for: 3_600 }),
    )
}

/// The demo link: the one scope, the one trust domain.
fn link() -> SyncScopeConfig {
    SyncScopeConfig::new(SCOPE, TRUST_DOMAIN)
}

/// Wall-clock seconds: the walk commands' logical time.
fn now_wall() -> LogicalTime {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Hex for a 32-byte id (event ids, report ids, conflict keys).
#[must_use]
pub fn hex_id(id: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in id {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Parse a hex id printed by one of these commands.
///
/// # Errors
///
/// On a wrong length or a non-hex digit.
pub fn parse_id(text: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        bail!("an id is 64 hex characters, got {}", bytes.len());
    }
    let mut out = [0u8; 32];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair).context("id is not ASCII")?;
        out[index] = u8::from_str_radix(pair, 16)
            .with_context(|| format!("id has a non-hex pair: {pair}"))?;
    }
    Ok(out)
}

fn config_path(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE)
}

fn read_config(dir: &Path) -> anyhow::Result<NodeConfig> {
    let path = config_path(dir);
    let bytes = std::fs::read(&path).with_context(|| {
        format!(
            "{} is not a walk node (no {}); run `id walk init` first",
            dir.display(),
            CONFIG_FILE
        )
    })?;
    serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))
}

/// How long an open waits for a node directory before concluding
/// another walk process holds it. One process uses a node
/// directory at a time (the store's own rule); without a bound,
/// opening a directory a `walk serve` holds waits forever.
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn open_timed_out(dir: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "opening {} timed out: another walk process is using this node directory \
         (stop its `id walk serve` first) — one process per node directory",
        dir.display()
    )
}

/// Open a node directory as a one-shot stored replica, plus its
/// recorded configuration. The caller shuts the store down on
/// every path out — success and error alike: a store dropped
/// without shutdown keeps its database locked, and the next open
/// of the directory waits on it.
async fn open_node(dir: &Path) -> anyhow::Result<(NodeConfig, DiskReplica)> {
    let config = read_config(dir)?;
    let stored = match tokio::time::timeout(
        OPEN_TIMEOUT,
        DiskReplica::open_dir(dir, replica(&config.name)),
    )
    .await
    {
        Ok(stored) => stored.with_context(|| format!("opening walk node {}", dir.display()))?,
        Err(_) => return Err(open_timed_out(dir)),
    };
    Ok((config, stored))
}

/// Create a walk node in `dir`, or check an existing one.
///
/// # Errors
///
/// On a name mismatch with an existing node, or a store failure.
pub async fn init_node(dir: &Path, name: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating node directory {}", dir.display()))?;
    let path = config_path(dir);
    if path.exists() {
        let existing = read_config(dir)?;
        if existing.name != name {
            bail!(
                "{} is already the walk node {:?}; refusing to rename it to {name:?}",
                dir.display(),
                existing.name
            );
        }
    } else {
        let config = NodeConfig {
            name: name.to_owned(),
        };
        std::fs::write(&path, serde_json::to_vec_pretty(&config)?)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    // Open once so the directory is a valid node (store + index)
    // before the first real command, and so a broken directory
    // fails here rather than there.
    let stored = match tokio::time::timeout(OPEN_TIMEOUT, DiskReplica::open_dir(dir, replica(name)))
        .await
    {
        Ok(stored) => stored.with_context(|| format!("creating the store in {}", dir.display()))?,
        Err(_) => return Err(open_timed_out(dir)),
    };
    stored.shutdown().await?;
    Ok(())
}

/// What a `write` produced: the record's address on the writing
/// node, its event id, and what the node's gate did with it.
#[derive(Debug)]
pub struct WrittenView {
    /// The record's `id://` address, minted with the writing
    /// node's name as resolver hint.
    pub address: String,
    /// The version's event id.
    pub event: EventId,
    /// The admission outcome.
    pub outcome: IngestOutcome,
}

/// Sign `text` as a demo note record by `writer`, following
/// `parents`: the substrate suites' `build_version`.
fn build_version(
    writer: &str,
    text: &str,
    parents: &[EventId],
    now: LogicalTime,
) -> anyhow::Result<(OpenedRecord, EventId, Envelope)> {
    let record = GraphRecord::new(
        LogicalGraphId::new(GRAPH),
        vec![id_core::graph::term::Triple::new(
            Term::typed(1, b"urn:note:1".to_vec()),
            Term::typed(2, b"body".to_vec()),
            Term::typed(3, text.as_bytes().to_vec()),
        )],
    );
    let signed = record.sign(CONTEXT, writer, &writer_key(writer))?;
    let event: EventId = *signed.id.content_id().digest();
    let header = VersionHeader {
        object_id: GRAPH,
        resolves: (parents.len() >= 2).then(|| conflict_key(parents)),
        parents: parents.to_vec(),
        body: signed.envelope.canonical_bytes()?,
    };
    let version = sign_envelope(
        &writer_key(writer),
        writer,
        SCOPE,
        &header.encode(),
        event,
        now,
    );
    Ok((signed, event, version))
}

/// Write a signed note record on the node in `dir`.
///
/// With no explicit `parents`, the record follows the node's
/// current heads for the demo graph — a plain successor write.
///
/// # Errors
///
/// On an unknown writer, a refused admission, or a store failure.
pub async fn write_record(
    dir: &Path,
    writer: &str,
    text: &str,
    parents: &[EventId],
    now: LogicalTime,
) -> anyhow::Result<WrittenView> {
    if !WRITERS.contains(&writer) {
        bail!("unknown demo writer {writer:?}; the walk writers are {WRITERS:?}");
    }
    let (config, mut stored) = open_node(dir).await?;
    // Every path out of an open node shuts its store down: a store
    // dropped without shutdown keeps its database locked, and the
    // next open of the directory would wait on it forever.
    let written = async {
        let parents: Vec<EventId> = if parents.is_empty() {
            stored.store().heads(&GRAPH)
        } else {
            parents.to_vec()
        };
        let (signed, event, version) = build_version(writer, text, &parents, now)?;
        let outcome = stored.produce(&version, now).await?;
        let address = IdUriV1::new(
            &config.name,
            NAMESPACE,
            UriTarget::Content(*signed.id.content_id()),
        )?
        .format();
        Ok::<_, anyhow::Error>(WrittenView {
            address,
            event,
            outcome,
        })
    }
    .await;
    stored.shutdown().await?;
    written
}

/// What a `read` found: the record's signer and its note text, if
/// the record carries a `body` triple.
#[derive(Debug)]
pub struct ReadView {
    /// The identity that signed the record.
    pub signer: String,
    /// The note's text (the `body` triple's object, lossy UTF-8).
    pub text: Option<String>,
}

/// Read the record `address` names from the node in `dir`.
///
/// The address's target names the record; the resolver hint names
/// the node that minted the address, and any node holding the
/// version may serve it — so the hint is not required to match
/// this node. Everything else the substrate suites check on the
/// way out is checked: the served bytes are the bytes the address
/// names, they open as a signed record, and the version carrying
/// them agrees about the record's id, object and signer.
///
/// # Errors
///
/// On a malformed address, a node that does not hold the version,
/// or served bytes that fail any of the checks.
pub async fn read_record(dir: &Path, address: &str) -> anyhow::Result<ReadView> {
    let uri = IdUriV1::parse(address).with_context(|| format!("parsing {address}"))?;
    let UriTarget::Content(named) = uri.target() else {
        bail!("{address} does not name a record");
    };
    let (_config, stored) = open_node(dir).await?;
    let served = stored.replica().envelope(named.digest()).cloned();
    stored.shutdown().await?;
    let served =
        served.ok_or_else(|| anyhow::anyhow!("this node does not hold the record at {address}"))?;

    let dispatch = ProfileDispatch::default();
    let header = VersionHeader::decode(&served.payload)
        .ok_or_else(|| anyhow::anyhow!("the serving version's header does not decode"))?;
    dispatch
        .verify_bytes(named, &header.body)
        .map_err(|_| anyhow::anyhow!("the served bytes are not the bytes the address names"))?;
    let envelope = RecordEnvelope::decode(&header.body)
        .map_err(|_| anyhow::anyhow!("the served bytes are not a signed record"))?;
    let opened = open(envelope, &dispatch)
        .map_err(|_| anyhow::anyhow!("the served record does not open"))?;
    if opened.id.content_id() != named {
        bail!("the serving version names a different record than the address");
    }
    if &header.object_id != opened.record.graph.as_bytes() {
        bail!("the serving version names a different object than the record");
    }
    if served.verifying_key != opened.envelope.body.key {
        bail!("the serving version names a different signer than the record");
    }
    let text = opened.assertions().find_map(|assertion| {
        let triple = assertion.triple;
        let Term::Typed { bytes, .. } = &triple.predicate else {
            return None;
        };
        if bytes != b"body" {
            return None;
        }
        let Term::Typed { bytes, .. } = &triple.object else {
            return None;
        };
        Some(String::from_utf8_lossy(bytes).into_owned())
    });
    Ok(ReadView {
        signer: opened.envelope.body.signer_id.clone(),
        text,
    })
}

/// One conflict report, as `reports` shows it.
#[derive(Debug)]
pub struct ReportView {
    /// The report's id.
    pub id: ReportId,
    /// Where the report stands.
    pub status: ReportStatus,
    /// The conflict key both sides of the dispute share.
    pub key: [u8; 32],
    /// The base the dispute diverged from.
    pub base: Vec<EventId>,
    /// The conflicting versions, with their signers.
    pub versions: Vec<(EventId, String)>,
}

/// A node's heads and reports for the demo graph.
#[derive(Debug)]
pub struct ReportsView {
    /// The graph's current heads.
    pub heads: Vec<EventId>,
    /// Every report the node holds, open and closed.
    pub reports: Vec<ReportView>,
}

/// Read a node's heads and conflict reports from its directory.
///
/// # Errors
///
/// On a store failure.
pub async fn node_reports(dir: &Path) -> anyhow::Result<ReportsView> {
    let (_config, stored) = open_node(dir).await?;
    let store = stored.store();
    let view = ReportsView {
        heads: store.heads(&GRAPH),
        reports: store
            .reports()
            .map(|(id, stored_report)| ReportView {
                id: *id,
                status: stored_report.status,
                key: stored_report.report.conflict_key(),
                base: stored_report.report.base.clone(),
                versions: stored_report
                    .report
                    .versions
                    .iter()
                    .map(|version| (version.event_id, version.signer.clone()))
                    .collect(),
            })
            .collect(),
    };
    stored.shutdown().await?;
    Ok(view)
}

/// The node's serving endpoint key: read from the node directory,
/// or generated and kept there on first serve.
fn endpoint_key(dir: &Path) -> anyhow::Result<SecretKey> {
    let path = dir.join(ENDPOINT_KEY_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("{} is not a 32-byte key", path.display()))?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            std::fs::write(&path, key.to_bytes())
                .with_context(|| format!("writing {}", path.display()))?;
            Ok(key)
        }
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// An endpoint for a walk node: no relay, no address lookup — the
/// peer token names the loopback socket directly.
async fn endpoint(dir: &Path) -> anyhow::Result<Endpoint> {
    Ok(Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .secret_key(endpoint_key(dir)?)
        .bind()
        .await?)
}

/// The loopback socket address an endpoint can be reached at.
fn loopback_sock(endpoint: &Endpoint) -> SocketAddr {
    let bound = endpoint.bound_sockets()[0];
    let ip = if bound.ip().is_unspecified() {
        Ipv4Addr::LOCALHOST.into()
    } else {
        bound.ip()
    };
    SocketAddr::new(ip, bound.port())
}

/// The peer token `serve` prints and `sync` dials:
/// `<endpoint-id>@<socket-address>`.
fn peer_token(endpoint: &Endpoint) -> String {
    format!("{}@{}", endpoint.id(), loopback_sock(endpoint))
}

/// Parse a peer token printed by `walk serve`.
///
/// # Errors
///
/// On a token that is not `<endpoint-id>@<socket-address>`.
pub fn parse_peer(token: &str) -> anyhow::Result<EndpointAddr> {
    let (id, sock) = token
        .split_once('@')
        .with_context(|| format!("peer token {token:?} is not <endpoint-id>@<socket-address>"))?;
    let id: iroh::EndpointId = id
        .parse()
        .with_context(|| format!("peer token endpoint id {id:?} does not parse"))?;
    let sock: SocketAddr = sock
        .parse()
        .with_context(|| format!("peer token socket address {sock:?} does not parse"))?;
    Ok(EndpointAddr::from_parts(id, [TransportAddr::Ip(sock)]))
}

/// Run one exchange from the node in `dir` to the serving peer at
/// `peer`, persisting what the node admits.
///
/// # Errors
///
/// On a store or exchange failure.
pub async fn sync_node(dir: &Path, peer: &str, now: LogicalTime) -> anyhow::Result<Exchange> {
    let target = parse_peer(peer)?;
    let (_config, mut stored) = open_node(dir).await?;
    let endpoint = endpoint(dir).await?;
    let exchange = stored
        .reconcile_remote_with(&endpoint, target, &link(), now)
        .await;
    endpoint.close().await;
    stored.shutdown().await?;
    Ok(exchange?)
}

/// A running walk server: the node in a directory, serving the
/// exchange protocol and persisting what its sessions admit.
pub struct WalkServer {
    /// The peer token a `walk sync` dials.
    pub peer: String,
    endpoint: Endpoint,
    router: Router,
    serving: ServingNode<FsBlobs>,
    persist: tokio::task::JoinHandle<anyhow::Result<()>>,
}

/// Start serving the node in `dir`.
///
/// # Errors
///
/// On a store or endpoint failure.
pub async fn start_server(dir: &Path) -> anyhow::Result<WalkServer> {
    let config = read_config(dir)?;
    let serving = match tokio::time::timeout(
        OPEN_TIMEOUT,
        ServingNode::open_dir(dir, replica(&config.name)),
    )
    .await
    {
        Ok(serving) => serving.with_context(|| format!("opening walk node {}", dir.display()))?,
        Err(_) => return Err(open_timed_out(dir)),
    };
    let (protocol, mut admissions) = serving.protocol();
    let endpoint = match endpoint(dir).await {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let _ = serving.shutdown().await;
            return Err(error);
        }
    };
    let router = Router::builder(endpoint.clone())
        .accept(EXCHANGE_ALPN, protocol)
        .spawn();
    let peer = peer_token(&endpoint);
    let persist_serving = serving.clone();
    let persist = tokio::spawn(async move {
        while let Some(session) = admissions.recv().await {
            persist_serving.persist_session(&session).await?;
        }
        Ok(())
    });
    Ok(WalkServer {
        peer,
        endpoint,
        router,
        serving,
        persist,
    })
}

impl WalkServer {
    /// Stop serving: end the router, drain and finish the persist
    /// loop, and shut the node's store down.
    ///
    /// # Errors
    ///
    /// On a persist failure during the node's life, or a store
    /// shutdown failure.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        let Self {
            endpoint,
            router,
            serving,
            persist,
            ..
        } = self;
        router
            .shutdown()
            .await
            .map_err(|error| anyhow::anyhow!("router shutdown failed: {error}"))?;
        endpoint.close().await;
        // The router held the protocol, and the protocol held the
        // admissions sender: with both gone the channel is closed
        // and the persist loop ends once it has drained.
        let persisted = persist
            .await
            .context("the persist loop panicked")?
            .context("persisting a session's admissions failed");
        serving.shutdown().await?;
        persisted
    }
}

/// Run a walk command.
pub async fn cmd_walk(command: WalkCommand) -> anyhow::Result<()> {
    match command {
        WalkCommand::Init { dir, name } => {
            init_node(&dir, &name).await?;
            println!("walk node {name:?} ready in {}", dir.display());
            Ok(())
        }
        WalkCommand::Write {
            dir,
            writer,
            text,
            parent,
        } => {
            let parents: Vec<EventId> = parent
                .iter()
                .map(|text| parse_id(text))
                .collect::<anyhow::Result<_>>()?;
            let written = write_record(&dir, &writer, &text, &parents, now_wall()).await?;
            println!("address: {}", written.address);
            println!("event:   {}", hex_id(&written.event));
            println!("outcome: {}", outcome_text(&written.outcome));
            Ok(())
        }
        WalkCommand::Read { dir, address } => {
            let read = read_record(&dir, &address).await?;
            println!("signer: {}", read.signer);
            match &read.text {
                Some(text) => println!("text:   {text}"),
                None => println!("text:   (no body triple)"),
            }
            Ok(())
        }
        WalkCommand::Reports { dir } => {
            let view = node_reports(&dir).await?;
            if view.heads.is_empty() {
                println!("heads: (none)");
            } else {
                for head in &view.heads {
                    println!("head: {}", hex_id(head));
                }
            }
            if view.reports.is_empty() {
                println!("reports: (none)");
            }
            for report in &view.reports {
                println!("report: {}", hex_id(&report.id));
                println!("  status: {}", status_text(&report.status));
                println!("  key:    {}", hex_id(&report.key));
                for base in &report.base {
                    println!("  base:   {}", hex_id(base));
                }
                for (event, signer) in &report.versions {
                    println!("  version: {} by {signer}", hex_id(event));
                }
            }
            Ok(())
        }
        WalkCommand::Serve { dir } => {
            let config = read_config(&dir)?;
            let server = start_server(&dir).await?;
            println!("WALK_NODE name={} peer={}", config.name, server.peer);
            println!(
                "serving walk node {:?} from {}; Ctrl-C to stop",
                config.name,
                dir.display()
            );
            tokio::signal::ctrl_c()
                .await
                .context("waiting for Ctrl-C")?;
            server.shutdown().await
        }
        WalkCommand::Sync { dir, peer } => {
            let exchange = sync_node(&dir, &peer, now_wall()).await?;
            if exchange.is_quiet() {
                println!("quiet: nothing to exchange");
                return Ok(());
            }
            print_transfer("sent to the peer", &exchange.forward);
            print_transfer("received from the peer", &exchange.backward);
            Ok(())
        }
    }
}

fn outcome_text(outcome: &IngestOutcome) -> String {
    match outcome {
        IngestOutcome::Applied => "applied".to_owned(),
        IngestOutcome::Conflict(report) => {
            format!("applied; conflict open in report {}", hex_id(report))
        }
        IngestOutcome::Resolved(report) => {
            format!("applied; resolved report {}", hex_id(report))
        }
        IngestOutcome::Held => "held until its parents are stored".to_owned(),
    }
}

fn status_text(status: &ReportStatus) -> String {
    match status {
        ReportStatus::Open => "open".to_owned(),
        ReportStatus::Resolved { by, .. } => format!("resolved by {}", hex_id(by)),
        ReportStatus::Superseded { by } => format!("superseded by {}", hex_id(by)),
    }
}

fn print_transfer(label: &str, transfer: &id_sync::federation::slices::b09_anti_entropy::Transfer) {
    for event in &transfer.offer.send {
        println!("{label}: {}", hex_id(event));
    }
    for (event, outcome) in &transfer.outcomes {
        match outcome {
            Ok(outcome) => println!("  {} -> {}", hex_id(event), outcome_text(outcome)),
            Err(error) => println!("  {} -> refused: {error:?}", hex_id(event)),
        }
    }
}
