//! Deterministic two-store derived-visibility oracle at graph-query
//! observables (#13583, slice 1 of queue task 21444).
//!
//! Oracle: **`visible_graph_oracle`** — the independent derivation below
//! folds a hand-declared edge table (which edges exist, and which of them
//! the restricted reader is authorized for) into the expected visible
//! nodes, edges and counts per reader profile, without calling the
//! production adapter. The test then compares the production adapter
//! ([`visible_friends`], [`visible_friend_count`], [`visible_friends_page`])
//! against the oracle on two stores, and compares the restricted reader's
//! observable result across the stores.
//!
//! Fixture: store A holds accounts Ann (the restricted reader), Bo and Cat
//! with friendships Ann–Bo and Bo–Cat. Store B holds the same authorized
//! material plus one restricted record: account Hid and friendship Bo–Hid,
//! neither of which the restricted reader is authorized for. The full-trust
//! peer (admin) is authorized for everything.
//!
//! Slice criterion: the restricted reader's authorized adjacency, count
//! and pagination at the same visible frontier are unchanged when the one
//! restricted record is added. A cursor naming the hidden neighbour steers
//! neither store. The full-trust view of store B still shows the hidden
//! edge, so the fixture is not vacuous.
//!
//! Scope: graph-query observables only. No search ranking, replication
//! inventory, cached derivatives, timing or physical-erasure claims; no
//! change to the #11185 presets and no projector work (#11186).
//!
//! Reader profiles under test: `Viewer::Account(ann)` (restricted reader),
//! `Viewer::Anonymous` (restricted: no friendship adjacency), and
//! `Viewer::Admin` (full-trust peer).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;
use id_directory::directory::Directory;
use id_directory::directory_view::{
    Viewer, visible_friend_count, visible_friends, visible_friends_page,
};

/// The reader profiles the oracle derives expectations for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reader {
    /// The restricted reader, by account ID: sees exactly its own adjacency.
    Restricted,
    /// An anonymous viewer: sees no friendship adjacency.
    Anonymous,
    /// The full-trust peer: sees every account's adjacency.
    FullTrust,
}

/// One declared edge of the model's edge table: the pair is friends, and
/// `authorized` says whether the restricted reader may observe it.
struct ModelEdge {
    a: &'static str,
    b: &'static str,
    authorized: bool,
}

/// The model's edge table. Store A holds the first two edges; store B adds
/// the restricted Bo–Hid edge. Only Ann–Bo is authorized for the
/// restricted reader; the remaining edges join accounts other than the
/// reader and are restricted records.
const MODEL_EDGES: &[ModelEdge] = &[
    ModelEdge {
        a: "ann",
        b: "bo",
        authorized: true,
    },
    ModelEdge {
        a: "bo",
        b: "cat",
        authorized: false,
    },
    ModelEdge {
        a: "bo",
        b: "hid",
        authorized: false,
    },
];

/// Oracle: `visible_graph_oracle` — derive the expected visible neighbours
/// of `account` for `reader` from the declared edge table, independently of
/// the production adapter. `edge_count` is how many table rows exist in the
/// store (2 for store A, 3 for store B). Full trust sees every declared
/// edge in range; the restricted reader sees only authorized edges incident
/// to its own account, and nothing of anyone else's adjacency; anonymous
/// sees nothing.
fn visible_graph_oracle(
    reader: Reader,
    account: &str,
    edge_count: usize,
    ids: &BTreeMap<&'static str, String>,
) -> Vec<String> {
    let mut neighbours = BTreeSet::new();
    for edge in &MODEL_EDGES[..edge_count] {
        let a = &ids[edge.a];
        let b = &ids[edge.b];
        match reader {
            Reader::FullTrust => {
                if account == a {
                    neighbours.insert(b.clone());
                } else if account == b {
                    neighbours.insert(a.clone());
                }
            }
            Reader::Restricted => {
                // Only the restricted reader's own account has authorized
                // adjacency, and only over authorized edges.
                let me = &ids["ann"];
                if account == me && edge.authorized {
                    if account == a {
                        neighbours.insert(b.clone());
                    } else if account == b {
                        neighbours.insert(a.clone());
                    }
                }
            }
            Reader::Anonymous => {}
        }
    }
    neighbours.into_iter().collect()
}

/// Deterministic account ID from a fixed seed, so both stores mint the same
/// IDs without randomness.
fn public_key(seed: u8) -> String {
    let key = SigningKey::from_bytes(&[seed; 32]);
    key.verifying_key()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Mint the four accounts with fixed IDs.
fn mint(directory: &mut Directory) -> BTreeMap<&'static str, String> {
    let mut ids = BTreeMap::new();
    for (slot, seed, name) in [
        ("ann", 11_u8, "Ann"),
        ("bo", 12_u8, "Bo"),
        ("cat", 13_u8, "Cat"),
        ("hid", 14_u8, "Hid"),
    ] {
        let id = public_key(seed);
        directory.sign_up_keyed(&id, name).unwrap();
        ids.insert(slot, id);
    }
    ids
}

fn befriend(directory: &mut Directory, a: &str, b: &str) {
    directory.request_friend_as(a, b).unwrap();
    directory.accept_friend_as(b, a).unwrap();
}

/// Build one store: the two base edges plus, for store B, the restricted
/// Bo–Hid edge.
fn build_store(extended: bool) -> (Directory, BTreeMap<&'static str, String>) {
    let mut directory = Directory::new();
    let ids = mint(&mut directory);
    befriend(&mut directory, &ids["ann"], &ids["bo"]);
    befriend(&mut directory, &ids["bo"], &ids["cat"]);
    if extended {
        befriend(&mut directory, &ids["bo"], &ids["hid"]);
    }
    (directory, ids)
}

/// Sort a neighbour list into account-ID order, the order the production
/// adapter promises.
fn sorted_vec(mut neighbours: Vec<String>) -> Vec<String> {
    neighbours.sort();
    neighbours
}

/// Walk every page of `account`'s visible adjacency at `limit`, returning
/// each page's neighbours and next-page flag.
fn walk_pages(
    viewer: &Viewer,
    directory: &Directory,
    account: &str,
    limit: usize,
) -> Vec<(Vec<String>, bool)> {
    let mut pages = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = visible_friends_page(viewer, directory, account, limit, after.as_deref());
        let cursor = page.neighbours.last().cloned();
        pages.push((page.neighbours, page.has_next));
        if !pages.last().expect("just pushed").1 {
            break;
        }
        after = cursor;
    }
    pages
}

#[test]
fn derived_visibility_restricted_adjacency_survives_hidden_record() {
    let (base, base_ids) = build_store(false);
    let (extended, extended_ids) = build_store(true);
    assert_eq!(
        base_ids, extended_ids,
        "both stores mint the same deterministic IDs"
    );
    let ids = base_ids;
    let (ann, bo, cat, hid) = (
        ids["ann"].clone(),
        ids["bo"].clone(),
        ids["cat"].clone(),
        ids["hid"].clone(),
    );
    let restricted = Viewer::Account(ann.clone());
    let anonymous = Viewer::Anonymous;
    let full_trust = Viewer::Admin;

    // The hidden record exists and the ungated adapter discloses it: the
    // raw adjacency of Bo differs across the stores. This is the leak the
    // viewer-gated adapter below closes, and it proves the fixture is not
    // vacuous.
    assert_eq!(
        sorted_vec(base.friends_of(&bo)),
        sorted_vec(vec![ann.clone(), cat.clone()]),
    );
    assert_eq!(
        sorted_vec(extended.friends_of(&bo)),
        sorted_vec(vec![ann.clone(), cat.clone(), hid.clone()]),
    );

    // The restricted reader's own adjacency matches the oracle on both
    // stores, and is unchanged by the hidden record.
    for (directory, edge_count) in [(&base, 2), (&extended, 3)] {
        assert_eq!(
            visible_friends(&restricted, directory, &ann),
            visible_graph_oracle(Reader::Restricted, &ann, edge_count, &ids),
        );
        assert_eq!(
            visible_friends(&restricted, directory, &ann),
            vec![bo.clone()],
        );
        assert_eq!(
            visible_friend_count(&restricted, directory, &ann),
            visible_graph_oracle(Reader::Restricted, &ann, edge_count, &ids).len(),
        );
    }
    assert_eq!(
        visible_friends(&restricted, &base, &ann),
        visible_friends(&restricted, &extended, &ann),
    );
    assert_eq!(
        visible_friend_count(&restricted, &base, &ann),
        visible_friend_count(&restricted, &extended, &ann),
    );

    // Someone else's adjacency is unauthorized for the restricted reader:
    // empty on both stores, matching the oracle — even though the raw
    // adjacency of Bo differs (see the leak witness above).
    for account in [&bo, &cat, &hid] {
        for (directory, edge_count) in [(&base, 2), (&extended, 3)] {
            assert_eq!(
                visible_friends(&restricted, directory, account),
                visible_graph_oracle(Reader::Restricted, account, edge_count, &ids),
            );
            assert!(visible_friends(&restricted, directory, account).is_empty());
            assert_eq!(visible_friend_count(&restricted, directory, account), 0);
        }
        assert_eq!(
            visible_friends(&restricted, &base, account),
            visible_friends(&restricted, &extended, account),
        );
    }

    // Anonymous sees no friendship adjacency on either store.
    for account in [&ann, &bo] {
        for (directory, edge_count) in [(&base, 2), (&extended, 3)] {
            assert_eq!(
                visible_friends(&anonymous, directory, account),
                visible_graph_oracle(Reader::Anonymous, account, edge_count, &ids),
            );
            assert!(visible_friends(&anonymous, directory, account).is_empty());
            assert_eq!(visible_friend_count(&anonymous, directory, account), 0);
        }
    }

    // Pagination at the same visible frontier agrees across the stores:
    // every page's neighbours and next-page flag, at several limits,
    // including limits past the end of the list.
    for limit in [1_usize, 2, 10] {
        assert_eq!(
            walk_pages(&restricted, &base, &ann, limit),
            walk_pages(&restricted, &extended, &ann, limit),
            "paged walk at limit {limit} differs across stores",
        );
        assert_eq!(
            walk_pages(&restricted, &base, &bo, limit),
            walk_pages(&restricted, &extended, &bo, limit),
        );
    }
    // The restricted walk over Ann's adjacency yields exactly the oracle
    // neighbours, one per page at limit 1, with the next-page flag set on
    // every page but the last.
    let oracle = visible_graph_oracle(Reader::Restricted, &ann, 3, &ids);
    let mut frontier: Option<String> = None;
    let mut walked = Vec::new();
    for (index, expected) in oracle.iter().enumerate() {
        let page = visible_friends_page(&restricted, &extended, &ann, 1, frontier.as_deref());
        assert_eq!(page.neighbours, vec![expected.clone()]);
        assert_eq!(page.has_next, index + 1 < oracle.len());
        walked.push(expected.clone());
        frontier = page.neighbours.into_iter().next();
    }
    assert_eq!(walked, oracle);

    // A cursor naming the hidden neighbour steers neither store: an empty
    // page with no next page, the same in both — and the same as any other
    // unknown cursor.
    let unknown = "00".repeat(32);
    for cursor in [hid.as_str(), unknown.as_str()] {
        for directory in [&base, &extended] {
            for account in [&ann, &bo] {
                let page =
                    visible_friends_page(&restricted, directory, account, 10, Some(cursor));
                assert!(page.neighbours.is_empty());
                assert!(!page.has_next);
            }
        }
    }
    assert_eq!(
        visible_friends_page(&restricted, &base, &ann, 10, Some(hid.as_str())),
        visible_friends_page(&restricted, &extended, &ann, 10, Some(hid.as_str())),
    );

    // The full-trust peer still sees the hidden edge in store B, matching
    // the full oracle on each store: acceptance proves the fixture kept the
    // record instead of dropping it.
    for (directory, edge_count) in [(&base, 2), (&extended, 3)] {
        for account in [&ann, &bo, &cat, &hid] {
            assert_eq!(
                visible_friends(&full_trust, directory, account),
                visible_graph_oracle(Reader::FullTrust, account, edge_count, &ids),
                "full-trust adjacency of {account} misses the oracle",
            );
            assert_eq!(
                visible_friend_count(&full_trust, directory, account),
                visible_graph_oracle(Reader::FullTrust, account, edge_count, &ids).len(),
            );
        }
    }
    assert_eq!(
        visible_friends(&full_trust, &base, &bo),
        sorted_vec(vec![ann.clone(), cat.clone()]),
    );
    assert_eq!(
        visible_friends(&full_trust, &extended, &bo),
        sorted_vec(vec![ann.clone(), cat.clone(), hid.clone()]),
    );
    assert_eq!(
        walk_pages(&full_trust, &extended, &bo, 1),
        vec![
            (vec![hid.clone()], true),
            (vec![ann.clone()], true),
            (vec![cat.clone()], false),
        ],
    );
}
