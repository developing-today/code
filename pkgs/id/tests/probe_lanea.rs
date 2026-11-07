// Probe (lane A, 2026-10-28): LocalEvents::record builds `after` from
// other_heads() = the head of EVERY other author in the store
// (local_events.rs:343-352,363), with no truncation, while
// chain::sign -> ChainExt::validate refuses after.len() > MAX_AFTER=32
// (chain.rs:33,187-189). With 33 other authors in the store, every local
// publish/set_tag/clear_tag must fail before anything is recorded.
// Asserts the DESIRED behaviour (a local set_tag succeeds with 33 other
// authors present), so it fails on current main and prints the error.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use p2panda_core::SigningKey;
use id::chain::{ChainExt, sign};
use id::event_store::EventStore;
use id::local_events::LocalEvents;
use id::tag_claims::{SET_KIND, TagOp};
use id::waiting::WaitingLimits;
use p2panda_core::Hash;

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

#[test]
fn probe_local_set_tag_with_33_other_authors() {
    let dir = tempfile::TempDir::new().unwrap();
    let subject = Hash::from_bytes([9; 32]);

    // Control: a fresh LocalEvents (no other authors) sets a tag fine.
    {
        let mut local = LocalEvents::open(dir.path(), key(250)).unwrap();
        let ok = local.set_tag(subject, "city", Some("Seattle"));
        println!("CTRL 0 other authors: set_tag ok = {:?}", ok.is_ok());
        assert!(ok.is_ok());
    }

    // Seed the same store with one seq-0 tag.set from each of 33 distinct
    // other authors (what syncing with 33 peers leaves behind).
    {
        let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 1_000).unwrap();
        for n in 1..=33u8 {
            let ext = ChainExt {
                kind: SET_KIND.to_owned(),
                group: None,
                after: Vec::new(),
                tag: Some(TagOp::Set {
                    subject,
                    name: "peer".to_owned(),
                    value: None,
                }),
                document: None,
                removes: None,
            };
            let event = sign(&key(n), 0, None, None, ext).unwrap();
            let outcome = store.ingest(&event.bytes, 1_000).unwrap();
            println!("seed author {n}: {outcome:?}");
        }
    }

    // Now the local node tries to record anything at all.
    let mut local = LocalEvents::open(dir.path(), key(250)).unwrap();
    let heads = local.frontier().unwrap();
    println!("OBSERVED frontier heads = {}", heads.len());
    let result = local.set_tag(subject, "city", Some("Tacoma"));
    match &result {
        Ok(_) => println!("OBSERVED set_tag SUCCEEDED with 33 other authors"),
        Err(e) => println!("OBSERVED set_tag FAILED: {e:#}"),
    }
    assert!(result.is_ok(), "DESIRED: local set_tag succeeds regardless of how many other authors the store holds");
}
