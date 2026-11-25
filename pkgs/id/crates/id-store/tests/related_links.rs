//! Tests for related path links in the sidecar payload (issue
//! #11419; design `doc/2026-11-17T22-00-00Z_design_id9114_related_path_links.md`,
//! sections 2–5, statements S3.21–S3.22).
//!
//! Oracle: **the design record's rules**. Fixed tests are the
//! vectors from the examples in design §3 (path-reference syntax)
//! and §5 (dangling-link behaviour), plus the §2 acceptance vector —
//! an identical link set gives an identical snapshot ID regardless
//! of input order — and a proptest asserting that canonical
//! ordering is invariant under input permutation.
//!
//! Why the modules are included by path: slice 9110-02 owns the
//! crate roots, so neither `sidecar_snapshot` nor `related_links`
//! is exported from id-store yet.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/related_links.rs"]
mod related_links;
#[path = "../src/sidecar_snapshot.rs"]
mod sidecar_snapshot;

use std::collections::BTreeMap;

use ed25519_dalek::SigningKey;
use proptest::prelude::*;

use id_core::canonical_cbor::{Value, decode_canonical, encode_canonical};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::location_ref::{LocationRef, R2Locator, TypedLocator};
use id_core::namespace_definition::NamespaceId;

use related_links::{
    LINKS_VIEW_KEY, LinkError, LinkResolution, LinkTarget, MAX_PATH_REF_BYTES, PathComparison,
    PathRef, RelatedLink, canonical_links, envelope_links, is_valid_relation, resolve_link,
    resolve_links, set_view_links, view_links,
};
use sidecar_snapshot::{SidecarBodyV1, SidecarEnvelopeV1, SnapshotId};

const PROFILE: HashProfile = HashProfile::RawBlake3_256;
const NAMESPACE: &str = "ns/alpha";
const TENANT: &str = "tenant/a";
const DOMAIN: &str = "id.sidecar/v1";
const SIGNER: &str = "sidecar-writer";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn subject() -> ContentId {
    ContentId::of_bytes(PROFILE, b"subject-1").unwrap()
}

fn content(seed: &[u8]) -> ContentId {
    ContentId::of_bytes(PROFILE, seed).unwrap()
}

fn snapshot(
    view: &BTreeMap<String, Value>,
    generation: u64,
    parent: Option<SnapshotId>,
    created_at: u64,
) -> SidecarEnvelopeV1 {
    let payload = sidecar_snapshot::encode_view(view).unwrap();
    let body = SidecarBodyV1::for_payload(
        subject(),
        NAMESPACE.to_owned(),
        TENANT.to_owned(),
        generation,
        parent,
        created_at,
        DOMAIN.to_owned(),
        SIGNER.to_owned(),
        key().verifying_key().to_bytes(),
        PROFILE,
        &payload,
    )
    .unwrap();
    SidecarEnvelopeV1::sign(body, payload, &key()).unwrap()
}

fn location_ref() -> LocationRef {
    let locator =
        TypedLocator::from(R2Locator::new("acct-1", "bucket-1", "objects/thing-1").unwrap());
    // A NamespaceId has its own grammar (a letter followed by
    // letters, digits or hyphens) — unlike the sidecar body's
    // free-text namespace label above.
    LocationRef::new(locator, NamespaceId::new("ns-alpha").unwrap(), TENANT).unwrap()
}

fn sample_links() -> Vec<RelatedLink> {
    vec![
        RelatedLink::new(
            "see-also",
            LinkTarget::ContentRef(content(b"target-content")),
        ),
        RelatedLink::new(
            "derived-from",
            LinkTarget::PathRef(PathRef::new("sources/original.md").unwrap()),
        ),
        RelatedLink::new(
            "see-also",
            LinkTarget::PathRef(PathRef::new("docs/guide.md").unwrap()),
        ),
        RelatedLink::new("located-at", LinkTarget::LocationRef(location_ref())),
        RelatedLink::new("part-of", LinkTarget::ContentRef(content(b"target-other"))),
    ]
}

fn snapshot_ref_link() -> RelatedLink {
    let genesis = snapshot(&BTreeMap::new(), 0, None, 1);
    RelatedLink::new(
        "supersedes",
        LinkTarget::SnapshotRef(genesis.snapshot_id().unwrap()),
    )
}

fn view_with(links: &[RelatedLink]) -> BTreeMap<String, Value> {
    let mut view = BTreeMap::new();
    view.insert(
        "title".to_owned(),
        Value::Text("Quarterly notes".to_owned()),
    );
    set_view_links(&mut view, links).unwrap();
    view
}

// ---------------------------------------------------------------- §3

#[test]
fn path_ref_accepts_namespace_relative_paths() {
    for raw in [
        "a",
        "docs/guide.md",
        "a/b/c",
        "a/b-c_d/e f.txt",
        "caf\u{e9}/menu",
        "deeply/nested/path/to/an/object.bin",
    ] {
        let path = PathRef::new(raw).unwrap();
        assert_eq!(path.as_str(), raw, "input is stored verbatim");
    }
}

#[test]
fn path_ref_keeps_the_authors_spelling_at_write() {
    // #14208 / A-9121.1: the stored text is never normalised. "cafe"
    // + combining acute (U+0301) stays decomposed; it does not become
    // "café" (U+00E9), which would silently redirect the link.
    let decomposed = PathRef::new("cafe\u{301}").unwrap();
    assert_eq!(decomposed.as_str(), "cafe\u{301}");
    let composed = PathRef::new("caf\u{e9}").unwrap();
    assert_eq!(composed.as_str(), "caf\u{e9}");
    // Without the namespace opt-in the two spellings name different
    // targets.
    assert_ne!(decomposed, composed);
    assert!(!decomposed.same_target(&composed, PathComparison::Exact));
    // With the opt-in they compare equal, and the stored spelling is
    // still the author's.
    assert!(decomposed.same_target(&composed, PathComparison::Nfc));
    assert_eq!(decomposed.comparison_key(PathComparison::Nfc), "caf\u{e9}");
    assert_eq!(decomposed.as_str(), "cafe\u{301}");
    // "A" + combining ring above (U+030A) is kept as written too.
    let path = PathRef::new("A\u{30a}/x").unwrap();
    assert_eq!(path.as_str(), "A\u{30a}/x");
    assert_eq!(path.comparison_key(PathComparison::Nfc), "\u{c5}/x");
    // The stored form re-validates to itself.
    assert_eq!(PathRef::new(path.as_str()).unwrap(), path);
}

#[test]
fn path_ref_rejects_leading_slash_and_bad_segments() {
    for raw in [
        "/absolute/path",
        "/",
        "",
        ".",
        "..",
        "./a",
        "../a",
        "a/./b",
        "a/../b",
        "a//b",
        "a/",
        "a/b/",
    ] {
        let error = PathRef::new(raw).unwrap_err();
        assert_eq!(
            error.class(),
            "invalid_path_ref",
            "input {raw:?} must be classed invalid_path_ref, got {error}"
        );
    }
}

#[test]
fn path_ref_cap_is_resource_limit_not_truncation() {
    let at_cap = "a".repeat(MAX_PATH_REF_BYTES);
    assert_eq!(
        PathRef::new(&at_cap).unwrap().as_str().len(),
        MAX_PATH_REF_BYTES
    );

    // A multi-segment path exactly at the cap is also fine.
    let segmented = format!("{}/{}", "a".repeat(2048), "b".repeat(2047));
    assert_eq!(segmented.len(), MAX_PATH_REF_BYTES);
    PathRef::new(&segmented).unwrap();

    let over_cap = "a".repeat(MAX_PATH_REF_BYTES + 1);
    let error = PathRef::new(&over_cap).unwrap_err();
    assert!(
        matches!(
            error,
            LinkError::ResourceLimit {
                limit: MAX_PATH_REF_BYTES,
                actual
            } if actual == MAX_PATH_REF_BYTES + 1
        ),
        "over-cap path must be ResourceLimit, got {error}"
    );
    assert_eq!(error.class(), "resource_limit");
}

// ---------------------------------------------------------------- §2

#[test]
fn target_kinds_round_trip_through_values() {
    let mut links = sample_links();
    links.push(snapshot_ref_link());
    for link in &links {
        let value = link.to_value();
        let bytes = encode_canonical(&value).unwrap();
        let decoded = decode_canonical(&bytes).unwrap();
        assert_eq!(RelatedLink::from_value(&decoded).unwrap(), *link);
        assert_eq!(
            link.target.kind(),
            match &link.target {
                LinkTarget::ContentRef(_) => "content",
                LinkTarget::SnapshotRef(_) => "snapshot",
                LinkTarget::PathRef(_) => "path",
                LinkTarget::LocationRef(_) => "location",
            }
        );
    }
}

#[test]
fn unknown_target_kind_is_unsupported_target_kind() {
    // An external URL and an opaque blob: both are new target kinds,
    // not path-link changes (design §4, Q1).
    for kind in ["url", "blob"] {
        let value = Value::Map(vec![
            (Value::Text("kind".to_owned()), Value::Text(kind.to_owned())),
            (
                Value::Text("url".to_owned()),
                Value::Text("https://example.com/x".to_owned()),
            ),
        ]);
        let error = LinkTarget::from_value(&value).unwrap_err();
        assert_eq!(error.class(), "unsupported_target_kind");
        assert!(
            matches!(error, LinkError::UnsupportedTargetKind { kind: ref got } if got == kind),
            "got {error}"
        );
    }
}

#[test]
fn unknown_relation_is_preserved_verbatim() {
    // Open vocabulary (design §4/Q5): readers keep unknown relations
    // and never fail the read — even one outside the vocabulary
    // shape.
    let link = RelatedLink::new(
        "x-custom-thing",
        LinkTarget::PathRef(PathRef::new("a/b").unwrap()),
    );
    let view = view_with(&[link.clone()]);
    assert_eq!(view_links(&view).unwrap(), vec![link]);

    let odd = RelatedLink::new(
        "SeeAlso",
        LinkTarget::ContentRef(content(b"target-content")),
    );
    let view = view_with(&[odd.clone()]);
    assert_eq!(view_links(&view).unwrap(), vec![odd]);

    assert!(is_valid_relation("see-also"));
    assert!(is_valid_relation("see_also2"));
    assert!(is_valid_relation("a"));
    assert!(!is_valid_relation("SeeAlso"));
    assert!(!is_valid_relation("9lives"));
    assert!(!is_valid_relation(""));
    assert!(!is_valid_relation("has space"));
}

#[test]
fn canonical_order_sorts_by_relation_then_target_bytes_and_collapses_duplicates() {
    let links = sample_links();
    let scrambled = vec![
        links[4].clone(),
        links[2].clone(),
        links[0].clone(),
        links[3].clone(),
        links[1].clone(),
        links[0].clone(), // exact duplicate of an existing link
    ];
    let canonical = canonical_links(&scrambled).unwrap();
    assert_eq!(canonical.len(), links.len(), "the duplicate collapses");
    // Relations in order: derived-from < located-at < part-of <
    // see-also (content) < see-also (path), the last pair ordered by
    // canonical target bytes.
    let relations: Vec<&str> = canonical.iter().map(|l| l.relation.as_str()).collect();
    assert_eq!(
        relations,
        [
            "derived-from",
            "located-at",
            "part-of",
            "see-also",
            "see-also"
        ]
    );
    assert!(matches!(canonical[3].target, LinkTarget::ContentRef(_)));
    assert!(matches!(canonical[4].target, LinkTarget::PathRef(_)));
    // Canonical form is a fixed point.
    assert_eq!(canonical_links(&canonical).unwrap(), canonical);
}

#[test]
fn identical_link_set_gives_identical_snapshot_id_regardless_of_input_order() {
    let links = sample_links();
    let mut reversed = links.clone();
    reversed.reverse();

    let view_a = view_with(&links);
    let view_b = view_with(&reversed);
    assert_eq!(
        sidecar_snapshot::encode_view(&view_a).unwrap(),
        sidecar_snapshot::encode_view(&view_b).unwrap(),
        "the same link set must give the same payload bytes"
    );

    let snap_a = snapshot(&view_a, 0, None, 42);
    let snap_b = snapshot(&view_b, 0, None, 42);
    assert_eq!(snap_a.snapshot_id().unwrap(), snap_b.snapshot_id().unwrap());
    snap_a.verify(&ProfileDispatch::default()).unwrap();
}

#[test]
fn adding_a_link_changes_the_snapshot_but_not_the_subject() {
    // Design §2 (S3.11, S3.12): links live in the payload, never in
    // the subject's content bytes.
    let bare = snapshot(&view_with(&[]), 0, None, 42);
    let linked = snapshot(&view_with(&sample_links()), 0, None, 42);
    assert_eq!(bare.body.subject_content_id, subject());
    assert_eq!(linked.body.subject_content_id, subject());
    assert_ne!(bare.snapshot_id().unwrap(), linked.snapshot_id().unwrap());
    assert_eq!(
        envelope_links(&linked).unwrap(),
        canonical_links(&sample_links()).unwrap()
    );
    assert!(envelope_links(&bare).unwrap().is_empty());
}

#[test]
fn empty_link_set_removes_the_view_entry() {
    let mut view = view_with(&sample_links());
    assert!(view.contains_key(LINKS_VIEW_KEY));
    set_view_links(&mut view, &[]).unwrap();
    assert!(!view.contains_key(LINKS_VIEW_KEY));
    assert!(view_links(&view).unwrap().is_empty());
    // A view that never had links reads as empty, not an error.
    assert!(view_links(&BTreeMap::new()).unwrap().is_empty());
}

// ---------------------------------------------------------------- §5

#[test]
fn dangling_link_is_stored_and_resolves_dangling() {
    // Writing never checks existence (design §5): the target is not
    // in any store this test holds, and the write still succeeds.
    let link = RelatedLink::new(
        "see-also",
        LinkTarget::ContentRef(content(b"never-replicated-here")),
    );
    let view = view_with(&[link.clone()]);
    assert_eq!(view_links(&view).unwrap(), vec![link.clone()]);

    // Resolving it is a distinct Dangling result carrying the
    // target — never an error, never a silent substitute.
    let resolution = resolve_link(&link, |_| false);
    assert!(resolution.is_dangling());
    assert_eq!(
        resolution,
        LinkResolution::Dangling {
            target: link.target.clone()
        }
    );
    assert_eq!(resolution.target(), &link.target);

    // The same link resolves when the caller can vouch for it.
    let resolution = resolve_link(&link, |target| *target == link.target);
    assert_eq!(
        resolution,
        LinkResolution::Resolved {
            target: link.target.clone()
        }
    );
}

#[test]
fn dangling_links_survive_rewrite() {
    // Readers preserve dangling links when they re-write a sidecar
    // (design §5; S3.14: the sidecar holds ALL metadata).
    let links = sample_links();
    let view = view_with(&links);
    let payload = sidecar_snapshot::encode_view(&view).unwrap();

    // Every link here is dangling under an empty existence oracle;
    // none may be pruned by reading and re-writing.
    let read = view_links(&sidecar_snapshot::decode_view(&payload).unwrap()).unwrap();
    assert!(
        resolve_links(&read, |_| false)
            .iter()
            .all(LinkResolution::is_dangling)
    );
    let mut rewritten = BTreeMap::new();
    rewritten.insert(
        "title".to_owned(),
        Value::Text("Quarterly notes".to_owned()),
    );
    set_view_links(&mut rewritten, &read).unwrap();
    assert_eq!(
        sidecar_snapshot::encode_view(&rewritten).unwrap(),
        payload,
        "a read/rewrite round trip must reproduce the payload exactly"
    );
    assert_eq!(read, canonical_links(&links).unwrap());
}

#[test]
fn resolution_is_per_target_and_read_only() {
    let links = sample_links();
    let present = content(b"target-content");
    let resolutions = resolve_links(
        &links,
        |target| matches!(target, LinkTarget::ContentRef(id) if *id == present),
    );
    assert_eq!(resolutions.len(), links.len());
    for (link, resolution) in links.iter().zip(&resolutions) {
        assert_eq!(resolution.target(), &link.target);
        let should_resolve = matches!(&link.target, LinkTarget::ContentRef(id) if *id == present);
        assert_eq!(!resolution.is_dangling(), should_resolve, "link {link:?}");
    }
    // Resolution changed nothing: the links still read back whole.
    let view = view_with(&links);
    assert_eq!(view_links(&view).unwrap(), canonical_links(&links).unwrap());
}

// ------------------------------------------------------------- proptest

fn link_pool() -> Vec<RelatedLink> {
    let mut pool = sample_links();
    pool.push(snapshot_ref_link());
    pool.push(RelatedLink::new(
        "see-also",
        LinkTarget::ContentRef(content(b"target-content")),
    ));
    pool
}

proptest! {
    /// Oracle: design §2 — canonical ordering is invariant under
    /// input permutation, so any shuffle of the same link multiset
    /// canonicalises identically and produces identical payload
    /// bytes (and therefore identical snapshot IDs).
    #[test]
    fn canonical_ordering_is_permutation_invariant(
        picks in proptest::collection::vec((0..4usize, 0..7usize), 0..12),
        swaps in proptest::collection::vec(any::<u8>(), 0..24),
    ) {
        let relations = ["see-also", "derived-from", "part-of", "located-at"];
        let pool = link_pool();
        let links: Vec<RelatedLink> = picks
            .iter()
            .map(|(r, t)| RelatedLink::new(relations[*r], pool[*t].target.clone()))
            .collect();

        // A deterministic Fisher–Yates shuffle driven by `swaps`.
        let mut shuffled = links.clone();
        if shuffled.len() > 1 && !swaps.is_empty() {
            for i in (1..shuffled.len()).rev() {
                let j = (swaps[i % swaps.len()] as usize) % (i + 1);
                shuffled.swap(i, j);
            }
        }

        prop_assert_eq!(canonical_links(&shuffled).unwrap(), canonical_links(&links).unwrap());
        let mut view_a = BTreeMap::new();
        let mut view_b = BTreeMap::new();
        set_view_links(&mut view_a, &links).unwrap();
        set_view_links(&mut view_b, &shuffled).unwrap();
        prop_assert_eq!(
            sidecar_snapshot::encode_view(&view_a).unwrap(),
            sidecar_snapshot::encode_view(&view_b).unwrap()
        );
    }
}
