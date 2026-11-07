//! Writer-set view projection and discovery over an explicitly pinned snapshot.
//! Tests the acceptance points of the writer-set design (§§4–5).

use id::idset::view::{expand_set, GroupMembers};
use id::groups::Role;
use id::idset::{Entry, Excludes, WriterSet};
use p2panda_core::Hash;

fn author(n: u8) -> [u8; 32] {
    [n; 32]
}

fn hash(n: u8) -> Hash {
    Hash::new([n; 32])
}

#[test]
fn test_expansion_public_exclusion_admits_nobody() {
    let set = WriterSet {
        set_id: hash(90),
        version: 3,
        owner: author(1),
        scope: Some("test".to_owned()),
        entries: vec![Entry::Public {
            include_history: true,
        }],
        excludes: Excludes(vec![Entry::Public {
            include_history: false,
        }]),
    };

    let expanded = expand_set(&set, &[], &hash(12), &[], false).unwrap();

    assert!(expanded.is_public());
    assert!(!expanded.contains(&author(2)));
}

#[test]
fn test_expansion_key_exclusion_wins() {
    let set = WriterSet {
        set_id: hash(90),
        version: 3,
        owner: author(1),
        scope: Some("test".to_owned()),
        entries: vec![Entry::Key {
            author: author(2),
            include_history: true,
        }],
        excludes: Excludes(vec![Entry::Key {
            author: author(2),
            include_history: false,
        }]),
    };

    let expanded = expand_set(&set, &[], &hash(12), &[], false).unwrap();

    assert!(expanded.contains(&author(2)));
    assert!(!expanded.contains(&author(3)));
}

#[test]
fn test_expansion_group_one_level_frontier_pinned() {
    let group = hash(10);
    let frontier = vec![hash(11)];
    let resolution = hash(12);
    let members = GroupMembers::new(
        group,
        frontier.clone(),
        resolution,
        std::collections::BTreeMap::from([
            (author(2), Role::Read),
            (author(3), Role::Write),
        ]),
    );

    let set = WriterSet {
        set_id: hash(90),
        version: 3,
        owner: author(1),
        scope: Some("test".to_owned()),
        entries: vec![Entry::Group {
            id: group,
            role_min: Role::Read,
            include_history: false,
        }],
        excludes: Excludes(vec![]),
    };

    let expanded = expand_set(&set, &frontier, &resolution, &[members], false).unwrap();

    assert!(expanded.contains(&author(2)));
    assert!(expanded.contains(&author(3)));
}

#[test]
fn test_expansion_retro_only_history_marked_entries() {
    let group = hash(10);
    let frontier = vec![hash(11)];
    let resolution = hash(12);
    let members = GroupMembers::new(
        group,
        frontier.clone(),
        resolution,
        std::collections::BTreeMap::from([
            (author(2), Role::Read),
            (author(3), Role::Write),
        ]),
    );

    let set = WriterSet {
        set_id: hash(90),
        version: 3,
        owner: author(1),
        scope: Some("test".to_owned()),
        entries: vec![
            Entry::Key {
                author: author(2),
                include_history: true,
            },
            Entry::Key {
                author: author(4),
                include_history: false,
            },
            Entry::Group {
                id: group,
                role_min: Role::Write,
                include_history: true,
            },
        ],
        excludes: Excludes(vec![Entry::Key {
            author: author(3),
            include_history: false,
        }]),
    };

    let expanded = expand_set(&set, &frontier, &resolution, &[members], true).unwrap();

    assert!(expanded.contains(&author(2)));
    assert!(!expanded.contains(&author(3)));
    assert!(!expanded.contains(&author(4)));
}
