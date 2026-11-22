//! B3 of the #9106 walking skeleton (#12232): the five steps of
//! `substrate_transport_path`, driven through the `id walk` command
//! surface instead of the test's own harness.
//!
//! Two walk nodes on disk (the B2(b) store), node B serving the
//! federation exchange protocol from the same process (the B2(c)
//! transport, composed with the store by `ServingNode`), and the
//! command functions of `id_cli::commands::walk` doing every step a
//! person at a terminal would: write a signed record at an `id://`
//! address on A, sync it to B, write conflicting edits on both,
//! read the conflict report on both, resolve on B, and read the
//! merged record back on A.
//!
//! Persistence across restart is not a separate scenario here: it
//! is the shape of the whole run. Every command opens its node
//! fresh from its directory and shuts the store down on the way
//! out, and B's server is stopped and restarted between stages —
//! so each stage's assertions are read back from disk after at
//! least one restart of the node that holds them. The oracles are
//! the path's own: differential (what B reads equals what A
//! signed; both nodes report the same conflict key, versions and
//! base) and metamorphic (a repeated sync is quiet).
//!
//! Venue: the sync rides QUIC/UDP on loopback, so this test runs
//! where UDP sends are permitted (the laptop, the CI nightly). The
//! Muse sandbox refuses UDP sends outright (EPERM); that is an
//! environment limit, not a verdict on the path. The test is part
//! of the `id.rust.substrate-path` registry entry, so the nightly
//! flake check (test-sandbox → `cargo nextest run --all-features`)
//! runs it with the other substrate suites.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use id_cli::commands::walk::{
    init_node, node_reports, read_record, start_server, sync_node, write_record,
};
use id_sync::federation::slices::b07_conflict_report::{IngestOutcome, ReportStatus};
use id_sync::federation::slices::b32_ingest_gate::EventId;

const START: u64 = 10_000;

fn heads(dir: &PathBuf) -> BTreeSet<EventId> {
    node_reports(dir).unwrap().heads.into_iter().collect()
}

#[tokio::test]
async fn the_walk_commands_drive_the_path_between_two_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    let a_dir = tmp.path().join("node-a");
    let b_dir = tmp.path().join("node-b");
    init_node(&a_dir, "node-a").unwrap();
    init_node(&b_dir, "node-b").unwrap();

    // Step 1–2: a signed record written at an id:// address on A,
    // held in A's on-disk store.
    let base = write_record(&a_dir, "alice", "first draft", &[], START).unwrap();
    assert_eq!(base.outcome, IngestOutcome::Applied);

    // Before any sync, B does not hold it: reading the address on B
    // is an error, not a fetch from anywhere else.
    assert!(read_record(&b_dir, &base.address).is_err());

    // Step 3: synced to B over the transport, B serving from its
    // own directory. The exchange names the one send.
    let server_b = start_server(&b_dir).await.unwrap();
    let exchange = sync_node(&a_dir, &server_b.peer, START + 1)
        .await
        .unwrap();
    assert_eq!(exchange.forward.offer.send, vec![base.event]);
    server_b.shutdown().await.unwrap();

    // After a restart of B, the record B admitted while serving is
    // still read from its address, and B's heads survived.
    let on_b = read_record(&b_dir, &base.address).unwrap();
    assert_eq!(on_b.signer, "alice");
    assert_eq!(on_b.text.as_deref(), Some("first draft"));
    assert_eq!(heads(&b_dir), BTreeSet::from([base.event]));

    // A repeated sync changes nothing.
    let server_b = start_server(&b_dir).await.unwrap();
    let exchange = sync_node(&a_dir, &server_b.peer, START + 2)
        .await
        .unwrap();
    assert!(exchange.is_quiet());
    server_b.shutdown().await.unwrap();

    // Step 4: conflicting writes — each writer follows the base
    // record on their own node. Locally each is a plain successor.
    let from_a = write_record(&a_dir, "alice", "alice's edit", &[base.event], START + 3).unwrap();
    let from_b = write_record(&b_dir, "bob", "bob's edit", &[base.event], START + 3).unwrap();
    assert_eq!(from_a.outcome, IngestOutcome::Applied);
    assert_eq!(from_b.outcome, IngestOutcome::Applied);
    assert_ne!(from_a.event, from_b.event);

    // Step 5: the next sync reports the conflict on both nodes.
    let server_b = start_server(&b_dir).await.unwrap();
    sync_node(&a_dir, &server_b.peer, START + 4).await.unwrap();
    server_b.shutdown().await.unwrap();

    let expected_heads = BTreeSet::from([from_a.event, from_b.event]);
    let mut report_ids = Vec::new();
    for dir in [&a_dir, &b_dir] {
        assert_eq!(heads(dir), expected_heads, "heads on {}", dir.display());
        let view = node_reports(dir).unwrap();
        let open: Vec<_> = view
            .reports
            .iter()
            .filter(|report| matches!(report.status, ReportStatus::Open))
            .collect();
        assert_eq!(open.len(), 1, "one open report on {}", dir.display());
        let report = open[0];
        assert_eq!(report.base, vec![base.event], "base on {}", dir.display());
        let named: BTreeSet<EventId> = report.versions.iter().map(|(id, _)| *id).collect();
        assert_eq!(named, expected_heads, "versions on {}", dir.display());
        let signers: BTreeSet<&str> = report.versions.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(signers, BTreeSet::from(["alice", "bob"]));
        report_ids.push((report.id, report.key));
    }
    // The differential oracle: both nodes report the same conflict —
    // same report id, same conflict key — after both have restarted.
    assert_eq!(report_ids[0], report_ids[1]);

    // Nothing was merged or dropped: both edits still read from
    // their addresses on B.
    assert_eq!(
        read_record(&b_dir, &from_a.address).unwrap().text.as_deref(),
        Some("alice's edit")
    );
    assert_eq!(
        read_record(&b_dir, &from_b.address).unwrap().text.as_deref(),
        Some("bob's edit")
    );

    // Resolution: bob writes a record that follows both edits, on
    // B; the outcome names the report it resolved.
    let merged = write_record(
        &b_dir,
        "bob",
        "both edits, merged by bob",
        &[from_a.event, from_b.event],
        START + 6,
    )
    .unwrap();
    assert_eq!(merged.outcome, IngestOutcome::Resolved(report_ids[1].0));

    // The next sync closes the report on A too, and says so.
    let server_b = start_server(&b_dir).await.unwrap();
    let exchange = sync_node(&a_dir, &server_b.peer, START + 7)
        .await
        .unwrap();
    server_b.shutdown().await.unwrap();
    assert_eq!(
        exchange.backward.outcomes,
        vec![(merged.event, Ok(IngestOutcome::Resolved(report_ids[0].0)))]
    );

    // After a restart of both nodes: no open report anywhere, one
    // head, and the merged record reads from its address on both.
    for dir in [&a_dir, &b_dir] {
        let view = node_reports(dir).unwrap();
        assert!(
            view.reports
                .iter()
                .all(|report| !matches!(report.status, ReportStatus::Open)),
            "no open report on {}",
            dir.display()
        );
        assert_eq!(view.heads, vec![merged.event], "heads on {}", dir.display());
        let read = read_record(dir, &merged.address).unwrap();
        assert_eq!(read.signer, "bob");
        assert_eq!(read.text.as_deref(), Some("both edits, merged by bob"));
    }

    // And a final sync is quiet.
    let server_b = start_server(&b_dir).await.unwrap();
    let exchange = sync_node(&a_dir, &server_b.peer, START + 8)
        .await
        .unwrap();
    assert!(exchange.is_quiet());
    server_b.shutdown().await.unwrap();
}
