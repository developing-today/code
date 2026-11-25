//! Talk-page posts and the per-object changelog (slice 1 of #5834, #14274).
//!
//! Covers the three invariants the slice must hold: records are immutable
//! (any changed byte changes identity or fails verification), every entry
//! has exactly one signing actor, and a chain verifies end to end from its
//! first entry or from a retained checkpoint, with a gap or a changed hash
//! detected on read.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::SigningKey;
use id_core::id_shape::{ContentId, HashProfile};
use id_store::talk_page::{
    ChainError, ChangeKind, ChangelogEntry, EntryDraft, EntryRef, PostDraft, Redaction,
    RedactionReason, RedactionTarget, Rendered, SignedEntry, TalkPageError, TalkPost, render_post,
    render_summary, verify_chain, verify_chain_bytes, verify_chain_to_head,
};

const OBJECT: &[u8] = b"object-1";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn post(body: &str, key: &SigningKey) -> TalkPost {
    TalkPost::sign(
        PostDraft {
            object_id: OBJECT.to_vec(),
            parent_post_id: None,
            supersedes: None,
            authored_at: 1_000,
            body: body.to_owned(),
        },
        key,
    )
    .unwrap()
}

/// Every kind but `redact`, which must name what it redacts.
const PLAIN_KINDS: [ChangeKind; 8] = [
    ChangeKind::Edit,
    ChangeKind::Fork,
    ChangeKind::Mark,
    ChangeKind::MergeProposal,
    ChangeKind::Merge,
    ChangeKind::Approval,
    ChangeKind::Reject,
    ChangeKind::Session,
];

/// A chain of `len` entries on [`OBJECT`], alternating two actors.
fn chain(len: u64) -> Vec<ChangelogEntry> {
    let mut entries: Vec<ChangelogEntry> = Vec::new();
    for seq in 0..len {
        let kind = PLAIN_KINDS[usize::try_from(seq).unwrap() % PLAIN_KINDS.len()];
        let draft = entries.last().map_or_else(
            || EntryDraft::genesis(OBJECT.to_vec(), kind, 10),
            |previous| EntryDraft::after(previous, kind, 10 + seq).unwrap(),
        );
        let actor = key(u8::try_from(seq % 2).unwrap() + 1);
        entries.push(SignedEntry::sign(draft, &actor).unwrap().receive(20 + seq));
    }
    entries
}

#[test]
fn post_id_is_the_hash_of_the_canonical_record() {
    let author = key(1);
    let first = post("hello", &author);
    first.verify().unwrap();
    let id = first.post_id().unwrap();
    assert_eq!(id.profile(), HashProfile::RawBlake3_256);
    id.verify(&first.record_bytes().unwrap()).unwrap();
    assert_eq!(first.author(), &author.verifying_key().to_bytes());
    assert_eq!(first.body(), Some("hello"));
    first.body_hash().verify(b"hello").unwrap();

    let decoded = TalkPost::from_canonical_bytes(&first.canonical_bytes().unwrap()).unwrap();
    assert_eq!(decoded, first);
    assert_eq!(decoded.post_id().unwrap(), id);

    assert_ne!(post("hello!", &author).post_id().unwrap(), id);
}

#[test]
fn receipt_time_stays_outside_the_post_id() {
    let first = post("hello", &key(1));
    let id = first.post_id().unwrap();
    let received = first.receive(5_000);
    assert_eq!(received.received_at(), 5_000);
    assert_eq!(received.post().authored_at(), 1_000);
    assert_eq!(received.post().post_id().unwrap(), id);
}

#[test]
fn a_changed_post_byte_fails_verification() {
    let bytes = post("hello", &key(1)).canonical_bytes().unwrap();
    let position = bytes
        .windows(5)
        .position(|window| window == b"hello")
        .unwrap();
    let mut tampered = bytes;
    tampered[position] = b'j';
    assert_eq!(
        TalkPost::from_canonical_bytes(&tampered),
        Err(TalkPageError::Invalid(
            "post body does not match its signed hash"
        ))
    );
}

#[test]
fn replies_and_supersession_reference_by_content_hash() {
    let author = key(1);
    let other = key(2);
    let root = post("question", &author);
    let reply = TalkPost::sign(
        PostDraft {
            object_id: OBJECT.to_vec(),
            parent_post_id: Some(root.post_id().unwrap()),
            supersedes: None,
            authored_at: 2_000,
            body: "answer".to_owned(),
        },
        &other,
    )
    .unwrap();
    assert_eq!(reply.parent_post_id(), Some(root.post_id().unwrap()));

    let correction = TalkPost::sign(
        PostDraft {
            object_id: OBJECT.to_vec(),
            parent_post_id: None,
            supersedes: Some(root.post_id().unwrap()),
            authored_at: 3_000,
            body: "better question".to_owned(),
        },
        &author,
    )
    .unwrap();
    correction.check_supersedes(&root).unwrap();
    // The superseded post is untouched and still verifies.
    root.verify().unwrap();

    let hijack = TalkPost::sign(
        PostDraft {
            object_id: OBJECT.to_vec(),
            parent_post_id: None,
            supersedes: Some(root.post_id().unwrap()),
            authored_at: 3_000,
            body: "not yours".to_owned(),
        },
        &other,
    )
    .unwrap();
    assert!(matches!(
        hijack.check_supersedes(&root),
        Err(TalkPageError::Invalid(_))
    ));
    assert!(matches!(
        correction.check_supersedes(&reply),
        Err(TalkPageError::Invalid(_))
    ));
}

#[test]
fn post_fields_are_validated() {
    let author = key(1);
    let draft = |object_id: &[u8], body: &str| PostDraft {
        object_id: object_id.to_vec(),
        parent_post_id: None,
        supersedes: None,
        authored_at: 1,
        body: body.to_owned(),
    };
    assert!(TalkPost::sign(draft(b"", "x"), &author).is_err());
    assert!(TalkPost::sign(draft(OBJECT, ""), &author).is_err());
    assert!(TalkPost::sign(draft(&[7; 129], "x"), &author).is_err());
    assert!(TalkPost::sign(draft(OBJECT, &"x".repeat(64 * 1024 + 1)), &author).is_err());
}

#[test]
fn every_kind_round_trips_its_token() {
    for kind in ChangeKind::ALL {
        assert_eq!(ChangeKind::from_token(kind.token()), Some(kind));
    }
    assert_eq!(ChangeKind::from_token("merge-proposal"), None);
}

#[test]
fn a_chain_verifies_end_to_end_from_its_first_entry() {
    let entries = chain(12);
    let head = verify_chain(OBJECT, None, &entries).unwrap().unwrap();
    assert_eq!(head.seq, 11);
    assert_eq!(head, entries[11].checkpoint().unwrap());
    assert_eq!(verify_chain(OBJECT, None, &[]), Ok(None));
}

#[test]
fn a_chain_replays_from_any_retained_checkpoint() {
    let entries = chain(8);
    let head = verify_chain(OBJECT, None, &entries).unwrap();
    for (split, entry) in entries.iter().enumerate() {
        let checkpoint = entry.checkpoint().unwrap();
        assert_eq!(
            verify_chain(OBJECT, Some(checkpoint), &entries[split + 1..]),
            Ok(head)
        );
    }
}

#[test]
fn a_missing_entry_is_a_gap() {
    let mut entries = chain(5);
    entries.remove(2);
    assert_eq!(
        verify_chain(OBJECT, None, &entries),
        Err(ChainError::Gap {
            expected: 2,
            found: 3
        })
    );
    // A chain without its first entry is a gap too.
    assert_eq!(
        verify_chain(OBJECT, None, &chain(3)[1..]),
        Err(ChainError::Gap {
            expected: 0,
            found: 1
        })
    );
}

#[test]
fn a_changed_entry_breaks_the_next_link() {
    let entries = chain(4);
    // Re-sign entry 1 with different content: it verifies on its own, but
    // entry 2 still names the original entry 1's hash.
    let forged = SignedEntry::sign(
        EntryDraft::after(&entries[0], ChangeKind::Merge, 999).unwrap(),
        &key(9),
    )
    .unwrap()
    .receive(21);
    let mut tampered = entries.clone();
    tampered[1] = forged;
    assert_eq!(
        verify_chain(OBJECT, None, &tampered),
        Err(ChainError::BrokenLink { seq: 2 })
    );

    // Changing only the receipt time also changes the entry hash.
    let mut late = entries;
    late[1] = late[1].signed().clone().receive(9_999);
    assert_eq!(
        verify_chain(OBJECT, None, &late),
        Err(ChainError::BrokenLink { seq: 2 })
    );
}

#[test]
fn a_changed_checkpoint_hash_is_detected() {
    let entries = chain(4);
    let mut checkpoint = entries[1].checkpoint().unwrap();
    checkpoint.hash = ContentId::new(HashProfile::RawBlake3_256, [0; 32]);
    assert_eq!(
        verify_chain(OBJECT, Some(checkpoint), &entries[2..]),
        Err(ChainError::BrokenLink { seq: 2 })
    );
}

#[test]
fn a_changed_or_dropped_last_entry_misses_the_retained_head() {
    let entries = chain(4);
    let head = entries[3].checkpoint().unwrap();
    assert_eq!(
        verify_chain_to_head(OBJECT, None, &entries, Some(head)),
        Ok(Some(head))
    );

    let mut tampered = entries.clone();
    tampered[3] = tampered[3].signed().clone().receive(1);
    assert!(matches!(
        verify_chain_to_head(OBJECT, None, &tampered, Some(head)),
        Err(ChainError::HeadMismatch { .. })
    ));
    assert!(matches!(
        verify_chain_to_head(OBJECT, None, &entries[..3], Some(head)),
        Err(ChainError::HeadMismatch { .. })
    ));
}

#[test]
fn a_foreign_entry_is_rejected() {
    let entries = chain(2);
    let foreign = SignedEntry::sign(
        EntryDraft {
            seq: 2,
            object_id: b"object-2".to_vec(),
            prev_hash: Some(entries[1].entry_hash().unwrap()),
            kind: ChangeKind::Edit,
            authored_at: 1,
            refs: Vec::new(),
            summary: None,
        },
        &key(1),
    )
    .unwrap()
    .receive(1);
    let mut mixed = entries;
    mixed.push(foreign);
    assert_eq!(
        verify_chain(OBJECT, None, &mixed),
        Err(ChainError::ForeignEntry { seq: 2 })
    );
}

#[test]
fn each_entry_has_exactly_one_signing_actor() {
    let actor = key(3);
    let user = key(4).verifying_key().to_bytes();
    let target = ContentId::of_bytes(HashProfile::RawBlake3_256, b"revision").unwrap();
    let entry = SignedEntry::sign(
        EntryDraft::genesis(OBJECT.to_vec(), ChangeKind::Edit, 1)
            .with_refs(vec![EntryRef::Content(target), EntryRef::OnBehalfOf(user)])
            .with_summary("fixed a typo"),
        &actor,
    )
    .unwrap();
    entry.verify().unwrap();
    assert_eq!(entry.actor(), &actor.verifying_key().to_bytes());
    assert_eq!(entry.on_behalf_of(), Some(&user));
    assert_eq!(entry.summary(), Some("fixed a typo"));
    entry
        .summary_hash()
        .unwrap()
        .verify(b"fixed a typo")
        .unwrap();

    // A proxy names at most one user it acts for.
    let two_users = EntryDraft::genesis(OBJECT.to_vec(), ChangeKind::Edit, 1).with_refs(vec![
        EntryRef::OnBehalfOf(user),
        EntryRef::OnBehalfOf(key(5).verifying_key().to_bytes()),
    ]);
    assert!(SignedEntry::sign(two_users, &actor).is_err());

    // An actor does not act on behalf of itself.
    let itself = EntryDraft::genesis(OBJECT.to_vec(), ChangeKind::Edit, 1)
        .with_refs(vec![EntryRef::OnBehalfOf(actor.verifying_key().to_bytes())]);
    assert!(SignedEntry::sign(itself, &actor).is_err());
}

#[test]
fn only_the_first_entry_has_no_previous_hash() {
    let actor = key(1);
    let mut orphan = EntryDraft::genesis(OBJECT.to_vec(), ChangeKind::Edit, 1);
    orphan.seq = 1;
    assert!(SignedEntry::sign(orphan, &actor).is_err());

    let mut linked_genesis = EntryDraft::genesis(OBJECT.to_vec(), ChangeKind::Edit, 1);
    linked_genesis.prev_hash = Some(ContentId::of_bytes(HashProfile::RawBlake3_256, b"x").unwrap());
    assert!(SignedEntry::sign(linked_genesis, &actor).is_err());

    let multi_line =
        EntryDraft::genesis(OBJECT.to_vec(), ChangeKind::Edit, 1).with_summary("two\nlines");
    assert!(SignedEntry::sign(multi_line, &actor).is_err());
}

#[test]
fn stored_chains_verify_on_read() {
    let entries = chain(6);
    let stored: Vec<Vec<u8>> = entries
        .iter()
        .map(|entry| entry.canonical_bytes().unwrap())
        .collect();
    for (bytes, entry) in stored.iter().zip(&entries) {
        assert_eq!(&ChangelogEntry::from_canonical_bytes(bytes).unwrap(), entry);
    }
    assert_eq!(
        verify_chain_bytes(OBJECT, None, &stored),
        verify_chain(OBJECT, None, &entries)
    );

    // A stored entry rewritten with another receipt time still decodes
    // and verifies on its own, and the next link catches it.
    let mut rewritten = stored.clone();
    rewritten[3] = entries[3]
        .signed()
        .clone()
        .receive(9_999)
        .canonical_bytes()
        .unwrap();
    assert_eq!(
        verify_chain_bytes(OBJECT, None, &rewritten),
        Err(ChainError::BrokenLink { seq: 4 })
    );

    // Bytes that no longer decode are reported by position.
    let mut truncated = stored.clone();
    truncated[3].pop();
    assert!(matches!(
        verify_chain_bytes(OBJECT, None, &truncated),
        Err(ChainError::Decode { index: 3, .. })
    ));

    // A dropped stored entry is a gap.
    let mut dropped = stored;
    dropped.remove(4);
    assert_eq!(
        verify_chain_bytes(OBJECT, None, &dropped),
        Err(ChainError::Gap {
            expected: 4,
            found: 5
        })
    );
}

#[test]
fn a_bad_signature_is_reported_with_its_sequence_number() {
    let entries = chain(3);
    let mut bytes = entries[1].canonical_bytes().unwrap();
    // The actor key is the first byte string in the map ("actor" sorts
    // first); corrupting it leaves the encoding valid.
    let position = bytes
        .windows(5)
        .position(|window| window == b"actor")
        .unwrap()
        + 5
        + 2;
    bytes[position] ^= 1;
    let decoded = ChangelogEntry::from_canonical_bytes(&bytes);
    assert!(matches!(
        decoded,
        Err(TalkPageError::BadSignature | TalkPageError::BadKey)
    ));
}

/// Append a `redact` entry for `target` to `entries`, signed by `moderator`.
fn redact(
    entries: &mut Vec<ChangelogEntry>,
    target: RedactionTarget,
    reason: RedactionReason,
    moderator: &SigningKey,
) -> ChangelogEntry {
    let entry = SignedEntry::sign(
        EntryDraft::after(entries.last().unwrap(), ChangeKind::Redact, 5_000)
            .unwrap()
            .with_redaction(Redaction { target, reason }),
        moderator,
    )
    .unwrap()
    .receive(6_000);
    entries.push(entry.clone());
    entry
}

#[test]
fn every_reason_round_trips_its_token() {
    for reason in RedactionReason::ALL {
        assert_eq!(RedactionReason::from_token(reason.token()), Some(reason));
    }
    assert_eq!(RedactionReason::from_token("author-request"), None);
}

#[test]
fn a_redacted_post_renders_as_a_tombstone_with_both_actors() {
    let author = key(1);
    let moderator = key(7);
    let original = post("my phone number is …", &author);
    let post_id = original.post_id().unwrap();
    let mut entries = chain(3);
    assert_eq!(
        render_post(&original, &entries),
        Ok(Rendered::Text("my phone number is …"))
    );

    let redaction = redact(
        &mut entries,
        RedactionTarget::Post(post_id),
        RedactionReason::Privacy,
        &moderator,
    );
    // The chain, now carrying the redaction, still verifies end to end.
    verify_chain(OBJECT, None, &entries).unwrap();

    // Dropping the body leaves the signed record and post id as they were.
    let stored = original.redacted(&redaction).unwrap();
    assert_eq!(stored.body(), None);
    stored.verify().unwrap();
    assert_eq!(stored.post_id().unwrap(), post_id);
    assert_eq!(stored.record_bytes(), original.record_bytes());
    let reread = TalkPost::from_canonical_bytes(&stored.canonical_bytes().unwrap()).unwrap();
    assert_eq!(reread, stored);

    let Ok(Rendered::Removed(tombstone)) = render_post(&stored, &entries) else {
        panic!("a redacted post renders as a tombstone");
    };
    assert_eq!(tombstone.target, RedactionTarget::Post(post_id));
    assert_eq!(tombstone.removed_hash, original.body_hash());
    assert_eq!(tombstone.original_actor, author.verifying_key().to_bytes());
    assert_eq!(tombstone.original_at, 1_000);
    assert_eq!(tombstone.redacted_by, moderator.verifying_key().to_bytes());
    assert_eq!(tombstone.redacted_at, 6_000);
    assert_eq!(tombstone.reason, RedactionReason::Privacy);
    // Readers that still hold the body render the same tombstone.
    assert_eq!(
        render_post(&original, &entries),
        Ok(Rendered::Removed(tombstone))
    );

    let shown = tombstone.to_string();
    assert!(shown.contains("post removed"), "{shown}");
    assert!(shown.contains(&hex_prefix(&author)), "{shown}");
    assert!(shown.contains(&hex_prefix(&moderator)), "{shown}");
    assert!(shown.contains("reason: privacy"), "{shown}");
}

fn hex_prefix(key: &SigningKey) -> String {
    hex::encode(&key.verifying_key().to_bytes()[..4])
}

#[test]
fn a_redacted_summary_keeps_the_chain_and_its_attribution() {
    let actor = key(2);
    let moderator = key(7);
    let mut entries = chain(1);
    entries.push(
        SignedEntry::sign(
            EntryDraft::after(&entries[0], ChangeKind::Edit, 30)
                .unwrap()
                .with_summary("renamed to something rude"),
            &actor,
        )
        .unwrap()
        .receive(31),
    );
    let target = RedactionTarget::Entry(entries[1].entry_hash().unwrap());
    let redaction = redact(&mut entries, target, RedactionReason::Abuse, &moderator);

    let mut stored = entries.clone();
    stored[1] = entries[1].summary_redacted(&redaction).unwrap();
    assert_eq!(stored[1].signed().summary(), None);
    assert_eq!(stored[1].entry_hash(), entries[1].entry_hash());
    // The chain verifies both before and after the summary is dropped,
    // also when read back from storage.
    let head = verify_chain(OBJECT, None, &entries).unwrap();
    assert_eq!(verify_chain(OBJECT, None, &stored), Ok(head));
    let bytes: Vec<Vec<u8>> = stored
        .iter()
        .map(|entry| entry.canonical_bytes().unwrap())
        .collect();
    assert_eq!(verify_chain_bytes(OBJECT, None, &bytes), Ok(head));

    let Ok(Some(Rendered::Removed(tombstone))) = render_summary(&stored[1], &stored) else {
        panic!("a redacted summary renders as a tombstone");
    };
    assert_eq!(tombstone.original_actor, actor.verifying_key().to_bytes());
    assert_eq!(tombstone.redacted_by, moderator.verifying_key().to_bytes());
    assert_eq!(tombstone.reason, RedactionReason::Abuse);
    // Everything but the summary text survives.
    assert_eq!(stored[1].signed().actor(), entries[1].signed().actor());
    assert_eq!(stored[1].signed().kind(), ChangeKind::Edit);

    // An entry that never had a summary renders none.
    assert_eq!(render_summary(&stored[0], &stored), Ok(None));
}

#[test]
fn redaction_rules_are_enforced() {
    let moderator = key(7);
    let original = post("hello", &key(1));
    let target = RedactionTarget::Post(original.post_id().unwrap());

    // A redact entry must name what it redacts, and only redact entries may.
    let entries = chain(2);
    let bare = EntryDraft::after(&entries[1], ChangeKind::Redact, 1);
    assert!(SignedEntry::sign(bare.unwrap(), &moderator).is_err());
    let smuggled = EntryDraft::after(&entries[1], ChangeKind::Edit, 1)
        .unwrap()
        .with_redaction(Redaction {
            target,
            reason: RedactionReason::Spam,
        });
    assert!(SignedEntry::sign(smuggled, &moderator).is_err());
    let twice = EntryDraft::after(&entries[1], ChangeKind::Redact, 1)
        .unwrap()
        .with_redaction(Redaction {
            target,
            reason: RedactionReason::Spam,
        })
        .with_redaction(Redaction {
            target,
            reason: RedactionReason::Abuse,
        });
    assert!(SignedEntry::sign(twice, &moderator).is_err());

    // A redaction of another post does not redact this one.
    let mut entries = chain(2);
    let other = post("other", &key(1));
    let unrelated = redact(
        &mut entries,
        RedactionTarget::Post(other.post_id().unwrap()),
        RedactionReason::Spam,
        &moderator,
    );
    assert!(original.redacted(&unrelated).is_err());
    assert_eq!(
        render_post(&original, &entries),
        Ok(Rendered::Text("hello"))
    );

    // A body cannot be dropped without a redaction covering it.
    let dropped = other.redacted(&unrelated).unwrap();
    assert!(render_post(&dropped, &chain(2)).is_err());

    // A redaction on another object's chain does not apply.
    let foreign = SignedEntry::sign(
        EntryDraft::genesis(b"object-2".to_vec(), ChangeKind::Redact, 1).with_redaction(
            Redaction {
                target,
                reason: RedactionReason::Legal,
            },
        ),
        &moderator,
    )
    .unwrap()
    .receive(1);
    assert!(original.redacted(&foreign).is_err());

    // A summary redaction names exactly the entry it redacts.
    let mut entries = chain(1);
    entries.push(
        SignedEntry::sign(
            EntryDraft::after(&entries[0], ChangeKind::Edit, 1)
                .unwrap()
                .with_summary("note"),
            &key(2),
        )
        .unwrap()
        .receive(2),
    );
    let target = RedactionTarget::Entry(entries[1].entry_hash().unwrap());
    let redaction = redact(&mut entries, target, RedactionReason::Other, &moderator);
    assert!(entries[1].summary_redacted(&redaction).is_ok());
    // It names entry 1, so it does not redact entry 0.
    assert!(entries[0].summary_redacted(&redaction).is_err());
}

#[test]
fn redact_entries_round_trip_through_storage() {
    let mut entries = chain(2);
    let target = RedactionTarget::Entry(entries[1].entry_hash().unwrap());
    let redaction = redact(
        &mut entries,
        target,
        RedactionReason::AuthorRequest,
        &key(7),
    );
    let decoded =
        ChangelogEntry::from_canonical_bytes(&redaction.canonical_bytes().unwrap()).unwrap();
    assert_eq!(decoded, redaction);
    assert_eq!(
        decoded.signed().redaction(),
        Some(Redaction {
            target,
            reason: RedactionReason::AuthorRequest,
        })
    );
}
