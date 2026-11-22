//! Run the #12232 walking-skeleton path from the CLI.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use id_core::graph::record::{GraphRecord, OpenedRecord, open};
use id_core::graph::term::{Term, Triple};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_uri_format::resolve_context;
use id_core::id_uri_parse::{IdUriV1, UriTarget};
use id_core::signed_envelope::{Envelope as RecordEnvelope, LogicalGraphId};
use id_sync::federation::exchange_net::{EXCHANGE_ALPN, ExchangeProtocol, reconcile_remote};
use id_sync::federation::record_store::DiskReplica;
use id_sync::federation::slices::b07_conflict_report::{
    ConflictStore, IngestOutcome, ObjectId, RetentionPolicy, StoredReport, VersionHeader,
    conflict_key,
};
use id_sync::federation::slices::b08_sync_scope::SyncScopeConfig;
use id_sync::federation::slices::b09_anti_entropy::Replica;
use id_sync::federation::slices::b32_ingest_gate::{
    Envelope, EventId, FreshnessWindow, IngressPath, SignerAuthorization, sign_envelope,
};
use iroh::{
    Endpoint, EndpointAddr, TransportAddr,
    endpoint::{RelayMode, presets},
    protocol::Router,
};
use tokio::sync::Mutex;

const START: u64 = 10_000;
const SCOPE: &str = "notes";
const TRUST_DOMAIN: &str = "home";
const NAMESPACE: &str = "notes";
const CONTEXT: &str = "namespace/notes";
const GRAPH: [u8; 32] = [0x11; 32];
const WRITERS: [(&str, u8); 2] = [("alice", 7), ("bob", 9)];
const ALICE: usize = 0;
const BOB: usize = 1;

struct Written {
    signed: OpenedRecord,
    event: EventId,
}

impl Written {
    fn address(&self, resolver: &str) -> Result<String> {
        Ok(IdUriV1::new(
            resolver,
            NAMESPACE,
            UriTarget::Content(*self.signed.id.content_id()),
        )?
        .format())
    }
}

fn key(writer: usize) -> SigningKey {
    SigningKey::from_bytes(&[WRITERS[writer].1; 32])
}

fn replica(name: &str) -> Replica {
    let signers = WRITERS
        .iter()
        .enumerate()
        .map(|(writer, (identity, _))| SignerAuthorization {
            identity: (*identity).to_owned(),
            verifying_key: key(writer).verifying_key().to_bytes(),
            scopes: [SCOPE.to_owned()].into_iter().collect(),
        })
        .collect();
    Replica::new(
        name,
        TRUST_DOMAIN,
        ConflictStore::new(
            signers,
            FreshnessWindow {
                max_age: 1_000_000,
                max_future_skew: 30,
            },
            RetentionPolicy { retain_for: 3_600 },
        ),
    )
}

fn note(text: &[u8]) -> GraphRecord {
    GraphRecord::new(
        LogicalGraphId::new(GRAPH),
        vec![Triple::new(
            Term::typed(1, b"urn:note:1".to_vec()),
            Term::typed(2, b"body".to_vec()),
            Term::typed(3, text.to_vec()),
        )],
    )
}

fn build_version(
    writer: usize,
    record: &GraphRecord,
    parents: &[EventId],
    now: u64,
) -> Result<(Written, Envelope)> {
    let identity = WRITERS[writer].0;
    let signed = record.sign(CONTEXT, identity, &key(writer))?;
    let event: EventId = *signed.id.content_id().digest();
    let object: ObjectId = *record.graph.as_bytes();
    let header = VersionHeader {
        object_id: object,
        resolves: (parents.len() >= 2).then(|| conflict_key(parents)),
        parents: parents.to_vec(),
        body: signed.envelope.canonical_bytes()?,
    };
    let version = sign_envelope(&key(writer), identity, SCOPE, &header.encode(), event, now);
    Ok((Written { signed, event }, version))
}

async fn publish(
    node: &mut DiskReplica,
    writer: usize,
    record: &GraphRecord,
    parents: &[EventId],
    now: u64,
) -> Result<Written> {
    let (written, version) = build_version(writer, record, parents, now)?;
    let outcome = node.produce(&version, now).await?;
    if outcome != IngestOutcome::Applied {
        bail!(
            "node {} did not apply the sample record: {outcome:?}",
            node.name()
        );
    }
    Ok(written)
}

fn read(node: &Replica, address: &str) -> Result<OpenedRecord> {
    let uri = IdUriV1::parse(address)?;
    resolve_context(uri.resolver_hint(), node.name())?;
    let UriTarget::Content(named) = uri.target() else {
        bail!("walking-skeleton address does not name record content");
    };
    let event: EventId = *named.digest();
    let version = node
        .envelope(&event)
        .context("node does not hold the record named by the address")?;
    let header = VersionHeader::decode(&version.payload).context("invalid version header")?;
    let dispatch = ProfileDispatch::default();
    dispatch.verify_bytes(named, &header.body)?;
    let opened = open(RecordEnvelope::decode(&header.body)?, &dispatch)?;
    if opened.id.content_id() != named
        || header.object_id != *opened.record.graph.as_bytes()
        || version.verifying_key != opened.envelope.body.key
    {
        bail!("record address, signed body, and version envelope do not join");
    }
    Ok(opened)
}

fn loopback_addr(endpoint: &Endpoint) -> Result<EndpointAddr> {
    let bound_sockets = endpoint.bound_sockets();
    let bound = bound_sockets
        .first()
        .context("iroh endpoint did not bind a socket")?;
    let ip = if bound.ip().is_unspecified() {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        bound.ip()
    };
    Ok(EndpointAddr::from_parts(
        endpoint.id(),
        [TransportAddr::Ip(SocketAddr::new(ip, bound.port()))],
    ))
}

async fn persist_network_admissions(
    stored: &mut DiskReplica,
    network: &Replica,
    source: &str,
    now: u64,
) -> Result<()> {
    let inventory = network.inventory();
    let events: Vec<EventId> = inventory
        .objects()
        .flat_map(|object| inventory.versions(object).into_iter().flatten().copied())
        .collect();
    for event in events {
        if stored.replica().envelope(&event).is_none() {
            let envelope = network
                .envelope(&event)
                .context("network inventory named a missing envelope")?;
            stored
                .receive(IngressPath::Internal, envelope, source, now)
                .await?;
        }
    }
    Ok(())
}

async fn exchange_over_iroh(a: &mut DiskReplica, b: &mut DiskReplica, now: u64) -> Result<()> {
    let endpoint_a = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;
    let endpoint_b = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;
    let b_network = Arc::new(Mutex::new(b.replica().clone()));
    let router = Router::builder(endpoint_b.clone())
        .accept(EXCHANGE_ALPN, ExchangeProtocol::new(b_network.clone()))
        .spawn();
    let mut a_network = a.replica().clone();
    let exchange = reconcile_remote(
        &endpoint_a,
        loopback_addr(&endpoint_b)?,
        &mut a_network,
        &SyncScopeConfig::new(SCOPE, TRUST_DOMAIN),
        now,
    )
    .await?;
    for (event, outcome) in exchange
        .forward
        .outcomes
        .iter()
        .chain(&exchange.backward.outcomes)
    {
        if let Err(error) = outcome {
            bail!("iroh exchange rejected version {event:02x?}: {error:?}");
        }
    }
    let b_snapshot = b_network.lock().await.clone();
    persist_network_admissions(a, &a_network, "node-b", now).await?;
    persist_network_admissions(b, &b_snapshot, "node-a", now).await?;
    router.shutdown().await?;
    endpoint_a.close().await;
    endpoint_b.close().await;
    Ok(())
}

fn open_report(node: &DiskReplica) -> Result<StoredReport> {
    let id = node
        .store()
        .open_report(&GRAPH)
        .context("node has no open conflict report")?;
    node.store()
        .report(&id)
        .cloned()
        .context("open report is missing its stored record")
}

fn heads(node: &DiskReplica) -> BTreeSet<EventId> {
    node.store().heads(&GRAPH).into_iter().collect()
}

/// Run the complete five-step path in private, durable demo nodes.
pub async fn cmd_substrate_path() -> Result<()> {
    let directory = tempfile::tempdir().context("creating isolated walking-skeleton directory")?;
    let mut a = DiskReplica::open_dir(&directory.path().join("node-a"), replica("node-a"))
        .await
        .context("opening durable node A")?;
    let mut b = DiskReplica::open_dir(&directory.path().join("node-b"), replica("node-b"))
        .await
        .context("opening durable node B")?;

    let base = publish(&mut a, ALICE, &note(b"first draft"), &[], START).await?;
    let base_uri_a = base.address("node-a")?;
    println!("1. Signed record on A: {base_uri_a}");
    if read(a.replica(), &base_uri_a)? != base.signed {
        bail!("node A did not read back the signed record it stored");
    }
    println!(
        "2. Stored on disk: node A holds {} version",
        heads(&a).len()
    );

    exchange_over_iroh(&mut a, &mut b, START + 1).await?;
    let base_uri_b = base.address("node-b")?;
    if read(b.replica(), &base_uri_b)? != base.signed {
        bail!("node B did not read the signed record synced over iroh");
    }
    println!("3. Synced to B over iroh: {base_uri_b}");

    let (from_a, version_a) =
        build_version(ALICE, &note(b"alice's edit"), &[base.event], START + 2)?;
    let outcome_a = a.produce(&version_a, START + 2).await?;
    if outcome_a != IngestOutcome::Applied {
        bail!("node A did not apply Alice's conflicting edit: {outcome_a:?}");
    }
    let (from_b, version_b) = build_version(BOB, &note(b"bob's edit"), &[base.event], START + 2)?;
    let outcome_b = b.produce(&version_b, START + 2).await?;
    if outcome_b != IngestOutcome::Applied || from_a.event == from_b.event {
        bail!("node B did not apply a distinct conflicting edit: {outcome_b:?}");
    }
    println!(
        "4. Conflicting edits written: A={} B={}",
        from_a.address("node-a")?,
        from_b.address("node-b")?
    );

    exchange_over_iroh(&mut a, &mut b, START + 3).await?;
    let report_a = open_report(&a)?;
    let report_b = open_report(&b)?;
    let expected_heads = BTreeSet::from([from_a.event, from_b.event]);
    if heads(&a) != expected_heads || heads(&b) != expected_heads {
        bail!("nodes disagree about the conflicting heads");
    }
    let expected_key = conflict_key(&[from_a.event, from_b.event]);
    for (name, report) in [("node A", &report_a), ("node B", &report_b)] {
        let versions: BTreeSet<EventId> = report
            .report
            .versions
            .iter()
            .map(|version| version.event_id)
            .collect();
        if report.report.conflict_key() != expected_key
            || report.report.object_id != GRAPH
            || report.report.base != [base.event]
            || versions != expected_heads
        {
            bail!(
                "{name} conflict report does not name the expected key, versions, and common base"
            );
        }
    }

    // Restart both durable nodes and make the same assertions without another exchange.
    a.shutdown().await.context("shutting down node A store")?;
    b.shutdown().await.context("shutting down node B store")?;
    drop(a);
    drop(b);
    let a = DiskReplica::open_dir(&directory.path().join("node-a"), replica("node-a"))
        .await
        .context("reopening durable node A")?;
    let b = DiskReplica::open_dir(&directory.path().join("node-b"), replica("node-b"))
        .await
        .context("reopening durable node B")?;
    if heads(&a) != expected_heads
        || heads(&b) != expected_heads
        || open_report(&a)? != report_a
        || open_report(&b)? != report_b
        || read(a.replica(), &from_b.address("node-a")?)? != from_b.signed
        || read(b.replica(), &from_a.address("node-b")?)? != from_a.signed
    {
        bail!("restart changed a head, conflict report, or served signed record");
    }
    println!(
        "5. Both nodes report conflict {} after restart ({} heads each)",
        expected_key
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        expected_heads.len()
    );
    a.shutdown()
        .await
        .context("closing reopened node A store")?;
    b.shutdown()
        .await
        .context("closing reopened node B store")?;
    Ok(())
}
