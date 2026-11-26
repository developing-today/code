//! Contract tests for the social object facade (issue #13095, slice
//! S3 of `doc/2026-11-24T05-55-00Z_design_id13095_social-object-envelope`).
//!
//! Oracle: the #13095 acceptance list and #9128 §§2–5, §7. Everything
//! here goes through the public path `id_core::social_object`, so the
//! tests fail if a type a caller needs stops being exported:
//!
//! - every one of the five profile tags round-trips through the strict
//!   canonical encoding, and equal envelopes encode to equal bytes;
//! - malformed envelopes fail closed, both as values and as bytes on
//!   the wire, with a stable failure class;
//! - envelopes read back from bytes feed the traversal, which hits
//!   each bound exactly, ends a parent cycle as a back-reference, and
//!   pages with a cursor without repeating or losing a position.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;

use id_core::canonical_cbor::{Value, decode_canonical, encode_canonical};
use id_core::id_shape::{ContentId, HashProfile};
use id_core::signed_envelope::EnvelopeId;
use id_core::social_object::{
    ALL_PROFILES, DEFAULT_MAX_DEPTH, DEFAULT_MAX_VISITED, EnvelopeError, MAX_AUTHOR_BYTES,
    MAX_ENVELOPE_BYTES, ObjectRef, PagingError, Requirement, SOCIAL_SCHEMA_VERSION, SocialObject,
    SocialObjectKind, TraversalEntry, TraversalMark, TraversalProfile, traverse, traverse_page,
};

fn oid(n: u16) -> ObjectRef {
    let mut digest = [0u8; 32];
    digest[..2].copy_from_slice(&n.to_be_bytes());
    ObjectRef::Envelope(EnvelopeId::from_content_id(ContentId::new(
        HashProfile::RawBlake3_256,
        digest,
    )))
}

fn content(n: u16) -> ObjectRef {
    let mut digest = [0xC0u8; 32];
    digest[..2].copy_from_slice(&n.to_be_bytes());
    ObjectRef::Content(ContentId::new(HashProfile::RawBlake3_256, digest))
}

fn object(id: u16, kind: SocialObjectKind, parent: Option<u16>) -> SocialObject {
    SocialObject {
        object_id: oid(id),
        kind,
        author: "alice".to_owned(),
        created_claim: u64::from(id),
        parent_ref: parent.map(oid),
        content_ref: content(id),
        schema_version: SOCIAL_SCHEMA_VERSION,
        succeeds_ref: None,
        target_ref: None,
    }
}

/// A minimal valid envelope of `kind`, meeting its profile's rules.
fn valid(kind: SocialObjectKind) -> SocialObject {
    let rule = kind.rule();
    let mut object = object(7, kind, None);
    if rule.parent == Requirement::Required {
        object.parent_ref = Some(oid(1));
    }
    if rule.target == Requirement::Required {
        object.target_ref = Some(oid(2));
    }
    object
}

/// Encode then decode, as a reader of the wire would see it.
fn through_wire(object: &SocialObject) -> SocialObject {
    let bytes = object.to_bytes().expect("valid envelope encodes");
    SocialObject::from_bytes(&bytes).expect("own encoding decodes")
}

#[test]
fn every_profile_tag_round_trips() {
    let tags: Vec<&str> = ALL_PROFILES.iter().map(|kind| kind.tag()).collect();
    assert_eq!(
        tags,
        ["feed-entry", "profile", "topic", "comment", "reaction"],
        "the five #9128 §2 profile tags, in tag order"
    );
    for kind in ALL_PROFILES {
        assert_eq!(SocialObjectKind::from_tag(kind.tag()), Some(kind));
        let object = valid(kind);
        object.validate().expect("valid envelope");
        let bytes = object.to_bytes().expect("encodes");
        assert!(bytes.len() <= MAX_ENVELOPE_BYTES);
        let back = SocialObject::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, object, "{} round-trips", kind.tag());
        assert_eq!(back.to_bytes().expect("re-encodes"), bytes);
        // The wire carries the tag as text under `profile`.
        let Value::Map(entries) = decode_canonical(&bytes).expect("canonical") else {
            panic!("envelope is a map");
        };
        assert!(entries.iter().any(|(key, value)| {
            *key == Value::Text("profile".to_owned())
                && *value == Value::Text(kind.tag().to_owned())
        }));
        // A version (a `succeeds_ref`) round-trips too.
        let mut version = object.clone();
        version.succeeds_ref = Some(oid(3));
        assert_eq!(through_wire(&version), version);
    }
    for tag in ["Topic", "feed_entry", "board", ""] {
        assert_eq!(SocialObjectKind::from_tag(tag), None, "{tag:?}");
    }
}

#[test]
fn equal_envelopes_encode_to_equal_bytes() {
    let first = valid(SocialObjectKind::Comment);
    let second = valid(SocialObjectKind::Comment);
    assert_eq!(first.to_bytes().unwrap(), second.to_bytes().unwrap());
    // The untrusted claim is still part of the signed shape: a
    // different claim is a different encoding.
    let mut later = valid(SocialObjectKind::Comment);
    later.created_claim += 1;
    assert_ne!(first.to_bytes().unwrap(), later.to_bytes().unwrap());
}

#[test]
fn malformed_envelopes_fail_closed() {
    let mut cases: Vec<(String, SocialObject, &str)> = Vec::new();
    let mut push = |name: &str, edit: &dyn Fn(&mut SocialObject), class: &'static str| {
        let mut object = valid(SocialObjectKind::Comment);
        edit(&mut object);
        cases.push((name.to_owned(), object, class));
    };
    push(
        "unsupported schema",
        &|o| o.schema_version = 2,
        "unsupported",
    );
    push("empty author", &|o| o.author.clear(), "malformed");
    push(
        "author too long",
        &|o| o.author = "a".repeat(MAX_AUTHOR_BYTES + 1),
        "malformed",
    );
    push(
        "control character in author",
        &|o| o.author = "al\nice".to_owned(),
        "malformed",
    );
    push(
        "own parent",
        &|o| o.parent_ref = Some(o.object_id),
        "relation",
    );
    push(
        "succeeds itself",
        &|o| o.succeeds_ref = Some(o.object_id),
        "relation",
    );
    // Every profile rule, broken one way at a time.
    for kind in ALL_PROFILES {
        let rule = kind.rule();
        let mut broken = valid(kind);
        broken.parent_ref = match rule.parent {
            Requirement::Required => None,
            Requirement::Forbidden => Some(oid(1)),
        };
        cases.push((format!("{} parent rule", kind.tag()), broken, "relation"));
        let mut broken = valid(kind);
        broken.target_ref = match rule.target {
            Requirement::Required => None,
            Requirement::Forbidden => Some(oid(2)),
        };
        cases.push((format!("{} target rule", kind.tag()), broken, "relation"));
    }
    // The author bound is inclusive.
    let mut longest = valid(SocialObjectKind::Comment);
    longest.author = "a".repeat(MAX_AUTHOR_BYTES);
    assert_eq!(through_wire(&longest), longest);

    for (name, object, class) in cases {
        let err = object.validate().expect_err(&name);
        assert_eq!(err.class(), class, "{name}: {err}");
        // No invalid envelope has an encoding.
        assert_eq!(object.to_bytes().expect_err(&name), err, "{name}");
    }
}

#[test]
fn malformed_wire_bytes_fail_closed() {
    let bytes = valid(SocialObjectKind::Topic).to_bytes().unwrap();
    let Value::Map(entries) = decode_canonical(&bytes).unwrap() else {
        panic!("envelope is a map");
    };
    let edited = |edit: &dyn Fn(&mut Vec<(Value, Value)>)| {
        let mut entries = entries.clone();
        edit(&mut entries);
        encode_canonical(&Value::Map(entries)).expect("test value encodes")
    };
    let set = |name: &'static str, value: Value| {
        move |entries: &mut Vec<(Value, Value)>| {
            for (key, slot) in entries.iter_mut() {
                if *key == Value::Text(name.to_owned()) {
                    *slot = value.clone();
                }
            }
        }
    };
    let remove = |name: &'static str| {
        move |entries: &mut Vec<(Value, Value)>| {
            entries.retain(|(key, _)| *key != Value::Text(name.to_owned()));
        }
    };
    let cases: Vec<(&str, Vec<u8>, EnvelopeError)> = vec![
        (
            "unsupported schema version",
            edited(&set("schema_version", Value::UInt(2))),
            EnvelopeError::UnsupportedSchema(2),
        ),
        (
            "unknown profile tag",
            edited(&set("profile", Value::Text("board".to_owned()))),
            EnvelopeError::UnknownProfile,
        ),
        (
            "unknown field",
            edited(&|entries| entries.push((Value::Text("mood".to_owned()), Value::Bool(true)))),
            EnvelopeError::Field {
                field: "envelope",
                problem: "carries an unknown field",
            },
        ),
        (
            "missing author",
            edited(&remove("author")),
            EnvelopeError::Field {
                field: "author",
                problem: "is missing",
            },
        ),
        (
            "missing parent for a topic",
            edited(&remove("parent_ref")),
            EnvelopeError::Relation {
                profile: "topic",
                rule: "requires a parent_ref",
            },
        ),
        (
            "author is not text",
            edited(&set("author", Value::UInt(7))),
            EnvelopeError::Field {
                field: "author",
                problem: "is not text",
            },
        ),
    ];
    for (name, wire, expected) in cases {
        assert_eq!(SocialObject::from_bytes(&wire), Err(expected), "{name}");
    }
    // Truncated, padded and oversized inputs are malformed.
    for (name, wire) in [
        ("truncated", bytes[..bytes.len() - 1].to_vec()),
        ("trailing byte", [bytes.clone(), vec![0]].concat()),
        ("oversized", vec![0; MAX_ENVELOPE_BYTES + 1]),
        ("not a map", encode_canonical(&Value::UInt(1)).unwrap()),
    ] {
        let err = SocialObject::from_bytes(&wire).expect_err(name);
        assert_eq!(err.class(), "malformed", "{name}: {err}");
    }
}

/// A topic whose thread has nested comments and reactions, and whose
/// own parent is one of its descendants, so the walk meets a cycle.
fn thread_on_the_wire() -> Vec<Vec<u8>> {
    use SocialObjectKind::{Comment, Reaction, Topic};
    let objects = [
        object(10, Topic, Some(14)),
        object(11, Comment, Some(10)),
        object(12, Comment, Some(11)),
        object(13, Reaction, Some(12)),
        object(14, Comment, Some(10)),
        object(15, Reaction, Some(14)),
        object(16, Comment, Some(14)),
        object(17, Comment, Some(16)),
        object(18, Reaction, Some(10)),
    ];
    objects.iter().map(|o| o.to_bytes().unwrap()).collect()
}

/// What a reader admits: the envelopes that decode and validate.
fn admitted(wire: &[Vec<u8>]) -> Vec<SocialObject> {
    wire.iter()
        .filter_map(|bytes| SocialObject::from_bytes(bytes).ok())
        .collect()
}

#[test]
fn validated_envelopes_feed_paged_traversal() {
    let mut wire = thread_on_the_wire();
    // A malformed envelope on the wire never reaches the walk.
    let mut bad_value = valid(SocialObjectKind::Comment).to_value().unwrap();
    if let Value::Map(entries) = &mut bad_value {
        for (key, slot) in entries.iter_mut() {
            if *key == Value::Text("author".to_owned()) {
                *slot = Value::Text(String::new());
            }
        }
    }
    wire.push(encode_canonical(&bad_value).unwrap());
    let objects = admitted(&wire);
    assert_eq!(objects.len(), 9, "the malformed envelope is refused");

    let root = oid(10);
    let all = |_: &SocialObject| true;
    let whole = traverse(&objects, &root, &TraversalProfile::default(), all);
    assert!(whole.complete);
    assert_eq!(whole.entries[0].object_id, root);
    assert_eq!(whole.entries[0].depth, 0);
    // The topic's parent is its own descendant: the cycle ends as a
    // back-reference to the root, not a second expansion.
    let back: Vec<&TraversalEntry> = whole
        .entries
        .iter()
        .filter(|entry| entry.mark == TraversalMark::BackReference)
        .collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].object_id, root);
    let expanded: HashSet<ObjectRef> = whole
        .entries
        .iter()
        .filter(|entry| entry.mark == TraversalMark::Expanded)
        .map(|entry| entry.object_id)
        .collect();
    assert_eq!(expanded.len(), 9);

    // Paging two expansions at a time gives the same walk, position
    // for position, with nothing repeated or lost.
    let small = TraversalProfile {
        max_visited: 2,
        ..TraversalProfile::default()
    };
    let mut paged = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let page = traverse_page(&objects, &root, &small, all, cursor).expect("page");
        assert!(page.missing.is_empty());
        let count = page
            .entries
            .iter()
            .filter(|entry| entry.mark == TraversalMark::Expanded)
            .count();
        assert!(count <= 2);
        paged.extend(page.entries);
        pages += 1;
        match page.cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(pages, 5);
    assert_eq!(paged, whole.entries);

    // A cursor is bound to its walk.
    let first = traverse_page(&objects, &root, &small, all, None).unwrap();
    let resume = first.cursor.expect("more to walk");
    assert_eq!(
        traverse_page(&objects, &oid(14), &small, all, Some(resume.clone())),
        Err(PagingError::CursorMismatch)
    );
    assert_eq!(
        traverse_page(
            &objects,
            &root,
            &TraversalProfile::default(),
            all,
            Some(resume)
        ),
        Err(PagingError::CursorMismatch)
    );

    // An unauthorized comment hides its whole subtree.
    let hidden = oid(14);
    let visible = traverse(&objects, &root, &TraversalProfile::default(), |o| {
        o.object_id != hidden
    });
    let seen: HashSet<ObjectRef> = visible.entries.iter().map(|e| e.object_id).collect();
    for gone in [14, 15, 16, 17] {
        assert!(
            !seen.contains(&oid(gone)),
            "{gone} is under the hidden comment"
        );
    }
    assert!(seen.contains(&oid(13)));
}

#[test]
fn traversal_hits_each_default_bound_exactly() {
    // Depth: a chain reaching one past the default display depth.
    let depth = u16::try_from(DEFAULT_MAX_DEPTH).unwrap();
    let mut chain = vec![object(0, SocialObjectKind::Profile, None)];
    for n in 1..=depth + 1 {
        chain.push(object(n, SocialObjectKind::Comment, Some(n - 1)));
    }
    let walk = traverse(&chain, &oid(0), &TraversalProfile::default(), |_| true);
    let last = walk.entries.last().unwrap();
    assert_eq!(
        walk.entries[usize::from(depth)].mark,
        TraversalMark::Expanded
    );
    assert_eq!(last.depth, DEFAULT_MAX_DEPTH + 1);
    assert_eq!(last.mark, TraversalMark::TruncatedDepth);
    assert!(!walk.complete);

    // Nodes: the root and its children fill the default budget, one
    // more child does not fit.
    let visited = u16::try_from(DEFAULT_MAX_VISITED).unwrap();
    let mut fan = vec![object(0, SocialObjectKind::Profile, None)];
    for n in 1..=visited {
        fan.push(object(n, SocialObjectKind::Comment, Some(0)));
    }
    let walk = traverse(&fan, &oid(0), &TraversalProfile::default(), |_| true);
    let marks = |mark| walk.entries.iter().filter(|e| e.mark == mark).count();
    assert_eq!(marks(TraversalMark::Expanded), DEFAULT_MAX_VISITED);
    assert_eq!(marks(TraversalMark::TruncatedBudget), 1);
    let page = traverse_page(&fan, &oid(0), &TraversalProfile::default(), |_| true, None).unwrap();
    assert_eq!(page.entries.len(), DEFAULT_MAX_VISITED);
    let rest = traverse_page(
        &fan,
        &oid(0),
        &TraversalProfile::default(),
        |_| true,
        page.cursor,
    )
    .unwrap();
    assert_eq!(rest.entries.len(), 1);
    assert!(rest.cursor.is_none());

    // Bytes: a budget of exactly three objects' cost expands three.
    let cost = 32 + 32 + "alice".len();
    let tight = TraversalProfile {
        max_bytes: 3 * cost,
        ..TraversalProfile::default()
    };
    let page = traverse_page(&chain, &oid(0), &tight, |_| true, None).unwrap();
    assert_eq!(page.entries.len(), 3);
    assert!(page.cursor.is_some());
    let too_small = TraversalProfile {
        max_bytes: cost - 1,
        ..TraversalProfile::default()
    };
    assert_eq!(
        traverse_page(&chain, &oid(0), &too_small, |_| true, None),
        Err(PagingError::ItemExceedsBudget)
    );
}
