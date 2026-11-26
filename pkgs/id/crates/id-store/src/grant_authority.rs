//! Selective-sharing grants at the object tier of the read order
//! (#14790 slice 2; #5799 decision record; #5678 order in
//! [`crate::read_authority`]).
//!
//! A live grant over a note is object-tier access the note's owner
//! gave. It is added alongside whatever the #5678 order resolves for
//! the note: a reader the resolved policy already admits keeps that
//! access, and a reader it refuses is admitted only by a grant that
//! covers them, the note, and `read`. A grant never removes access the
//! order gives, and it never makes an unknown note readable: an
//! unknown note is denied before any grant is looked at, so the answer
//! never confirms that a note exists (design invariant 6).
//!
//! Only grants whose delegation chain is rooted in a grant issued by
//! the note's owner count ([`GrantSet::authorize_rooted_in`]). A grant
//! someone else issued over the note, or delegated from such a grant,
//! admits nobody.
//!
//! Keys: an account is named by the lowercase hex of its key
//! ([`identity_key`]), a note or a group by its hash's hex
//! ([`Hash::to_hex`]). A reader's groups are the group subjects in the
//! grant set that the host's [`GroupRoles`] resolver says the reader
//! belongs to, so the host never lists memberships separately.
//!
//! This module only reads. Serving a `snapshot_once` grant's snapshot,
//! and recording it with [`GrantSet::consume_snapshot`], is the
//! caller's step; persisting grants is a later slice of #14790. Notes
//! carry no stable collection here, so only object-scoped grants match.

use std::collections::BTreeSet;

use p2panda_core::Hash;

use id_core::waiting::AuthorKey;
use id_grants::{AccessRequest, Decision, GrantSet, Operation, Requester, Subject};

use crate::note_checkpoints::{GroupRoles, NoteCheckpoints};
use crate::read_authority::{ReadDefaults, may_read};

/// The grant-model identity for an account key: its lowercase hex.
#[must_use]
pub fn identity_key(author: AuthorKey) -> String {
    hex::encode(author)
}

/// The grant-model [`Requester`] for `reader`: their identity key and
/// every group subject in `grants` that `groups` says they belong to.
#[must_use]
pub fn requester_for(reader: AuthorKey, groups: &GroupRoles, grants: &GrantSet) -> Requester {
    let member_of: BTreeSet<String> = grants
        .grants()
        .filter_map(|grant| match &grant.subject {
            Subject::Group(id) => Some(id),
            Subject::Identity(_) | Subject::Public => None,
        })
        .filter(|id| parse_hash(id).is_some_and(|group| groups.role(group, reader).is_some()))
        .cloned()
        .collect();
    Requester {
        identity: identity_key(reader),
        groups: member_of,
    }
}

/// Parses a 32-byte hex id; `None` for anything else.
fn parse_hash(id: &str) -> Option<Hash> {
    let bytes: [u8; 32] = hex::decode(id).ok()?.try_into().ok()?;
    Some(Hash::from_bytes(bytes))
}

/// Whether `reader` may read `note` under the #5678 order with the
/// owner's live grants added at the object tier, as of `now` (Unix
/// seconds, UTC). Unknown notes are denied.
#[must_use]
pub fn may_read_with_grants(
    notes: &NoteCheckpoints,
    note: Hash,
    reader: AuthorKey,
    groups: &GroupRoles,
    defaults: &ReadDefaults,
    grants: &GrantSet,
    now: u64,
) -> bool {
    let Some((owner, _)) = notes.access(note) else {
        return false;
    };
    if may_read(notes, note, reader, groups, defaults) {
        return true;
    }
    let requester = requester_for(reader, groups, grants);
    let object = note.to_hex();
    let request = AccessRequest {
        requester: Some(&requester),
        object: &object,
        collection: None,
        operation: Operation::Read,
        now,
    };
    matches!(
        grants.authorize_rooted_in(&request, &identity_key(owner)),
        Decision::Allow(_)
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::documents::DocumentRef;
    use crate::note_checkpoints::{NoteAccess, NoteRole};
    use id_grants::{Delegation, Duration, Grant, GrantId, Scope};

    const NOW: u64 = 1_800_000_000;

    fn hash(byte: u8) -> Hash {
        Hash::from_bytes([byte; 32])
    }

    fn author(byte: u8) -> AuthorKey {
        [byte; 32]
    }

    fn content(bytes: &[u8]) -> DocumentRef {
        DocumentRef::for_bytes("text/markdown", bytes).unwrap()
    }

    /// A log with one note, `hash(10)`, owned by `author(1)` and
    /// scoped so nobody but the owner reads it.
    fn private_note() -> NoteCheckpoints {
        let mut log = NoteCheckpoints::new();
        log.create_with_access(
            hash(10),
            author(1),
            content(b"private"),
            NoteAccess::Scoped(vec![]),
        )
        .unwrap();
        log
    }

    fn read_grant(name: &str, issuer: AuthorKey, subject: Subject, note: Hash) -> Grant {
        Grant {
            id: GrantId(name.to_owned()),
            issuer: identity_key(issuer),
            subject,
            scopes: BTreeSet::from([Scope::Object(note.to_hex())]),
            operations: BTreeSet::from([Operation::Read]),
            duration: Duration::Standing,
            delegation: Delegation::default(),
            parent: None,
        }
    }

    fn check(log: &NoteCheckpoints, note: Hash, reader: AuthorKey, grants: &GrantSet) -> bool {
        may_read_with_grants(
            log,
            note,
            reader,
            &GroupRoles::none(),
            &ReadDefaults::none(),
            grants,
            NOW,
        )
    }

    #[test]
    fn test_an_owner_grant_admits_a_reader_the_policy_refuses() {
        let log = private_note();
        let mut grants = GrantSet::new();
        assert!(!check(&log, hash(10), author(2), &grants));
        grants
            .insert_root(read_grant(
                "g1",
                author(1),
                Subject::Identity(identity_key(author(2))),
                hash(10),
            ))
            .unwrap();
        assert!(check(&log, hash(10), author(2), &grants));
        // The grant names one reader; another stays refused.
        assert!(!check(&log, hash(10), author(3), &grants));
        // The owner reads with or without grants.
        assert!(check(&log, hash(10), author(1), &GrantSet::new()));
    }

    #[test]
    fn test_a_grant_someone_else_issued_admits_nobody() {
        let log = private_note();
        let mut grants = GrantSet::new();
        grants
            .insert_root(read_grant(
                "g1",
                author(2),
                Subject::Identity(identity_key(author(2))),
                hash(10),
            ))
            .unwrap();
        assert!(!check(&log, hash(10), author(2), &grants));
    }

    #[test]
    fn test_revocation_and_expiry_take_effect_at_the_next_check() {
        let log = private_note();
        let mut grants = GrantSet::new();
        let mut expiring = read_grant(
            "g1",
            author(1),
            Subject::Identity(identity_key(author(2))),
            hash(10),
        );
        expiring.duration = Duration::ExpiresAt(NOW + 10);
        grants.insert_root(expiring).unwrap();
        let reads_at = |grants: &GrantSet, now| {
            may_read_with_grants(
                &log,
                hash(10),
                author(2),
                &GroupRoles::none(),
                &ReadDefaults::none(),
                grants,
                now,
            )
        };
        assert!(reads_at(&grants, NOW));
        assert!(!reads_at(&grants, NOW + 11));
        grants.revoke(&GrantId("g1".to_owned()));
        assert!(!reads_at(&grants, NOW));
    }

    #[test]
    fn test_an_unknown_note_stays_denied_even_under_a_public_grant() {
        let log = private_note();
        let mut grants = GrantSet::new();
        grants
            .insert_root(read_grant("g1", author(1), Subject::Public, hash(99)))
            .unwrap();
        assert!(!check(&log, hash(99), author(2), &grants));
    }

    #[test]
    fn test_a_public_grant_admits_any_reader_of_that_note_only() {
        let mut log = private_note();
        log.create_with_access(
            hash(11),
            author(1),
            content(b"other"),
            NoteAccess::Scoped(vec![]),
        )
        .unwrap();
        let mut grants = GrantSet::new();
        grants
            .insert_root(read_grant("g1", author(1), Subject::Public, hash(10)))
            .unwrap();
        assert!(check(&log, hash(10), author(7), &grants));
        assert!(!check(&log, hash(11), author(7), &grants));
    }

    #[test]
    fn test_a_group_grant_admits_members_the_resolver_names() {
        let log = private_note();
        let group = hash(50);
        let mut grants = GrantSet::new();
        grants
            .insert_root(read_grant(
                "g1",
                author(1),
                Subject::Group(group.to_hex()),
                hash(10),
            ))
            .unwrap();
        let groups = GroupRoles::new(move |g, member| {
            (g == group && member == author(4)).then_some(NoteRole::Read)
        });
        let reads = |reader| {
            may_read_with_grants(
                &log,
                hash(10),
                reader,
                &groups,
                &ReadDefaults::none(),
                &grants,
                NOW,
            )
        };
        assert!(reads(author(4)));
        assert!(!reads(author(5)));
        assert_eq!(
            requester_for(author(4), &groups, &grants).groups,
            BTreeSet::from([group.to_hex()])
        );
    }

    #[test]
    fn test_a_grant_never_removes_access_the_order_gives() {
        let mut log = NoteCheckpoints::new();
        log.create(hash(12), author(1), content(b"open")).unwrap();
        // No recorded policy, no defaults: public. No grants at all.
        assert!(check(&log, hash(12), author(8), &GrantSet::new()));
    }
}
