//! R2 adapter tests for the #9133 benchmark contract
//! (slice C9133-02-R2, issue #10163). Test-first companion to
//! `src/adapters/r2.rs`; no network, no provider calls — the Fake
//! R2 capability fixture is the system under test.
//!
//! Oracle (reference): the Fake R2 capability fixture verifies
//! conditional writes, opaque generation-qualified locator
//! construction and never-reuse; conditional DeleteObject is
//! unsupported. A stale delete targets only its captured old key
//! and cannot alias a new generation.

use id_benchmarks::adapters::r2::{FakeR2, R2Adapter, R2Error};
use id_benchmarks::{content_id, support_level};

const BYTES: &[u8] = b"r2-adapter-fixture-bytes";

#[test]
fn conditional_writes_are_verified() {
    let mut r2 = FakeR2::new();
    // Put-if-absent: first writer wins, second is rejected.
    let etag = r2.put_if_absent("k/1", BYTES).expect("first put-if-absent");
    assert_eq!(etag, content_id(BYTES));
    assert_eq!(
        r2.put_if_absent("k/1", BYTES),
        Err(R2Error::ConditionFailed {
            key: "k/1".to_string()
        })
    );
    // Put-if-match: wrong etag rejected, right etag accepted.
    assert_eq!(
        r2.put_if_match("k/1", b"other", "sha256:wrong"),
        Err(R2Error::ConditionFailed {
            key: "k/1".to_string()
        })
    );
    let etag2 = r2
        .put_if_match("k/1", b"other", &etag)
        .expect("put-if-match with the served etag");
    assert_eq!(etag2, content_id(b"other"));
    // Copy-if-match follows the same conditional surface.
    assert!(r2.copy_if_match("k/1", "k/2", &etag2).is_ok());
    assert_eq!(
        r2.copy_if_match("k/1", "k/3", "sha256:wrong"),
        Err(R2Error::ConditionFailed {
            key: "k/1".to_string()
        })
    );
    // Native counters describe exactly the mix driven above.
    let counters = r2.counters();
    assert_eq!(counters.conditional_accepted, 3);
    assert_eq!(counters.conditional_rejected, 3);
    assert_eq!(counters.copies, 1);
}

#[test]
fn get_and_checksum_roundtrip() {
    let mut adapter = R2Adapter::new();
    let locator = adapter.put_object(BYTES).expect("put");
    assert_eq!(adapter.get_object(&locator).expect("get"), BYTES);
    assert!(adapter.checksum_matches(&locator).expect("checksum"));
    let counters = adapter.counters();
    assert_eq!(counters.gets, 1);
    assert_eq!(counters.checksums, 1);
    // A locator for absent content does not resolve.
    let missing = adapter.put_object(b"gone").expect("put second");
    adapter.tombstone(&missing.content_id).expect("tombstone");
    adapter.prove_owner_fence();
    assert!(adapter.delete_captured_key(&missing).expect("delete"));
    assert_eq!(
        adapter.get_object(&missing),
        Err(R2Error::NotFound {
            key: missing.key.clone()
        })
    );
}

#[test]
fn locators_are_generation_qualified_and_keys_never_reused() {
    let mut adapter = R2Adapter::new();
    let gen1 = adapter.put_object(BYTES).expect("put gen 1");
    assert_eq!(gen1.generation, 1);
    assert!(gen1.key.ends_with("/gen-1"), "key: {}", gen1.key);
    assert!(gen1.key.contains(&gen1.content_id[7..15]));
    // Idempotent re-put of live content: same locator, no new key.
    assert_eq!(adapter.put_object(BYTES).expect("re-put"), gen1);
    // Resurrection after tombstone mints the next generation under
    // a fresh key — the gen-1 key is never reused.
    adapter.tombstone(&gen1.content_id).expect("tombstone");
    let gen2 = adapter.put_object(BYTES).expect("resurrect");
    assert_eq!(gen2.generation, 2);
    assert!(gen2.key.ends_with("/gen-2"), "key: {}", gen2.key);
    assert_ne!(gen2.key, gen1.key);
}

#[test]
fn conditional_delete_is_unsupported() {
    let mut adapter = R2Adapter::new();
    let locator = adapter.put_object(BYTES).expect("put");
    assert_eq!(
        adapter.delete_conditional(&locator, &locator.content_id),
        Err(R2Error::Unsupported {
            operation: "conditional DeleteObject".to_string()
        })
    );
    // Nothing was deleted and the refusal is counted natively.
    assert!(adapter.get_object(&locator).is_ok());
    assert_eq!(adapter.counters().unsupported, 1);
    // The plan's capability mapping agrees: r2 tombstone delete is
    // fence-gated, never a plain conditional delete.
    assert_eq!(
        support_level(
            "object_delete_captured_key",
            "r2",
            "Plain DeleteObject on captured never-reused key"
        ),
        "plain-delete-captured-never-reused-key-after-fence"
    );
    assert_eq!(
        support_level("object_tombstone_delete", "r2", "delete the object"),
        "unsupported"
    );
}

#[test]
fn delete_requires_the_owner_fence() {
    let mut adapter = R2Adapter::new();
    let locator = adapter.put_object(BYTES).expect("put");
    adapter.tombstone(&locator.content_id).expect("tombstone");
    // Captured, but the #9084 owner fence is not proven: refused.
    match adapter.delete_captured_key(&locator) {
        Err(R2Error::DeleteRefused { reason }) => {
            assert!(reason.contains("owner fence"), "reason: {reason}");
        }
        other => panic!("expected DeleteRefused, got {other:?}"),
    }
    assert!(adapter.get_object(&locator).is_ok());
    assert_eq!(adapter.counters().deletes_refused, 1);
    // With the fence proven, the same captured delete succeeds.
    adapter.prove_owner_fence();
    assert!(adapter.delete_captured_key(&locator).expect("delete"));
    assert_eq!(adapter.counters().deletes, 1);
}

#[test]
fn stale_delete_targets_only_its_captured_old_key() {
    let mut adapter = R2Adapter::new();
    let gen1 = adapter.put_object(BYTES).expect("put gen 1");
    adapter
        .tombstone(&gen1.content_id)
        .expect("tombstone gen 1");
    // A new generation of the same content goes live.
    let gen2 = adapter.put_object(BYTES).expect("put gen 2");
    assert_eq!(gen2.content_id, gen1.content_id);
    adapter.prove_owner_fence();
    // The stale (gen-1) delete removes only the captured old key.
    assert!(adapter.delete_captured_key(&gen1).expect("stale delete"));
    assert_eq!(
        adapter.get_object(&gen1),
        Err(R2Error::NotFound {
            key: gen1.key.clone()
        })
    );
    // The new generation is untouched and still checksums clean —
    // the stale delete did not alias it.
    assert_eq!(adapter.get_object(&gen2).expect("get gen 2"), BYTES);
    assert!(adapter.checksum_matches(&gen2).expect("checksum gen 2"));
    // The live current generation itself is not a captured key and
    // cannot be plain-deleted through this path.
    match adapter.delete_captured_key(&gen2) {
        Err(R2Error::DeleteRefused { .. }) => {}
        other => panic!("expected DeleteRefused for live key, got {other:?}"),
    }
}
