//! Default tag views authorize clear markers by claim author or the subject group's settled Admin.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id::chain::{ChainExt, Event, sign};
use id::event_store::{EventStore, Ingest};
use id::groups::event::encode as encode_group_event;
use id::groups::{Body as GroupBody, DeviceOp, Event as GroupEvent, Role as GroupRole};
use id::membership::AuthId;
use id::membership::body::encode_body;
use id::tag_claims::{CLEAR_KIND, SET_KIND, TagOp};
use id::waiting::WaitingLimits;
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Hash, SeqNum, SigningKey};

const GROUP_OP: &str = "group.op";

fn key(who: char) -> SigningKey {
    SigningKey::from_bytes(&[who as u8; 32])
}

fn author(who: char) -> [u8; 32] {
    *key(who).verifying_key().as_bytes()
}

#[derive(Default)]
struct Chains {
    last: BTreeMap<char, (SeqNum, Hash)>,
}

impl Chains {
    fn sign(
        &mut self,
        who: char,
        group: Option<Hash>,
        after: &[Hash],
        kind: &str,
        tag: Option<TagOp>,
        body: Option<Vec<u8>>,
    ) -> EventWithBody {
        let (seq, backlink) = match self.last.get(&who) {
            Some((seq, id)) => (seq + 1, Some(*id)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: kind.to_owned(),
            group,
            after: after.to_vec(),
            tag,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,

            chat: None,
        };
        let event = sign(&key(who), seq, backlink, body.as_deref(), ext).unwrap();
        self.last.insert(who, (seq, event.id));
        EventWithBody { event, body }
    }

    fn create_group(&mut self) -> EventWithBody {
        let action = GroupAction::Create {
            // The creator is listed as Manage so the event-store bridge can represent the create;
            // the settled group fold gives the creator the distinct Admin role.
            initial_members: ['A', 'B']
                .iter()
                .map(|who| {
                    (
                        GroupMember::Individual(AuthId::Key(author(*who))),
                        Access::manage(),
                    )
                })
                .collect(),
        };
        self.sign(
            'A',
            None,
            &[],
            GROUP_OP,
            None,
            Some(encode_body(&action).unwrap()),
        )
    }

    fn set_tag(&mut self, who: char, subject: Hash, after: &[Hash]) -> EventWithBody {
        self.sign(
            who,
            None,
            after,
            SET_KIND,
            Some(TagOp::Set {
                subject,
                name: "label".to_owned(),
                value: Some("value".to_owned()),
            }),
            None,
        )
    }

    /// A device operation by `who` in `group`, carried as a `group.event` body — the
    /// channel a device grant or revocation travels to the store by (issue #5656).
    fn device_op(&mut self, who: char, group: Hash, after: &[Hash], op: DeviceOp) -> EventWithBody {
        let wire = GroupEvent {
            id: Hash::digest(format!("device op by {who}: {op:?}").as_bytes()),
            author: author(who),
            after: after.to_vec(),
            group: Some(group),
            body: GroupBody::Device(op),
        };
        let body = encode_group_event(&wire).unwrap();
        self.sign(who, Some(group), after, "group.event", None, Some(body))
    }

    fn clear(&mut self, who: char, claims: &[Hash], after: &[Hash]) -> EventWithBody {
        self.sign(
            who,
            None,
            after,
            CLEAR_KIND,
            Some(TagOp::Clear {
                claims: claims.to_vec(),
            }),
            None,
        )
    }
}

struct EventWithBody {
    event: Event,
    body: Option<Vec<u8>>,
}

fn deliver(store: &mut EventStore, sent: &EventWithBody) {
    let result = store
        .ingest_with_body(&sent.event.bytes, sent.body.as_deref(), 0)
        .unwrap();
    assert!(
        matches!(result, Ingest::Accepted { .. }),
        "event was not applied: {result:?}"
    );
}

fn open() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

fn grouped_subject_scenario(clearer: char) -> (tempfile::TempDir, EventStore, Hash, Hash) {
    let (dir, mut store) = open();
    let mut chains = Chains::default();
    let group_create = chains.create_group();
    let group = group_create.event.id;
    let subject = chains.sign('B', Some(group), &[group], "test.subject", None, None);
    let claim = chains.set_tag('B', subject.event.id, &[subject.event.id]);
    let clear = chains.clear(clearer, &[claim.event.id], &[claim.event.id]);

    for event in [&group_create, &subject, &claim, &clear] {
        deliver(&mut store, event);
    }
    (dir, store, claim.event.id, clear.event.id)
}

/// B's claim on a grouped subject, then a clear signed by the device key `D`, which A —
/// the group's Admin creator — authorized at `scope`. When `revoke` is set, A revokes the
/// device again before the clear.
fn device_clear_scenario(scope: GroupRole, revoke: bool) -> (tempfile::TempDir, EventStore) {
    let (dir, mut store) = open();
    let mut chains = Chains::default();
    let group_create = chains.create_group();
    let group = group_create.event.id;
    let subject = chains.sign('B', Some(group), &[group], "test.subject", None, None);
    let claim = chains.set_tag('B', subject.event.id, &[subject.event.id]);
    let grant = chains.device_op(
        'A',
        group,
        &[group],
        DeviceOp::Add {
            device: author('D'),
            scope,
        },
    );
    deliver(&mut store, &group_create);
    deliver(&mut store, &subject);
    deliver(&mut store, &claim);
    deliver(&mut store, &grant);
    if revoke {
        let revoke = chains.device_op(
            'A',
            group,
            &[grant.event.id],
            DeviceOp::Revoke {
                device: author('D'),
            },
        );
        deliver(&mut store, &revoke);
    }
    let clear = chains.clear('D', &[claim.event.id], &[claim.event.id]);
    deliver(&mut store, &clear);
    (dir, store)
}

#[test]
fn test_an_admin_scoped_device_of_an_admin_may_clear_another_authors_tag() {
    // The device's effective role is the authorizing account's Admin role capped by the
    // grant's Admin scope: Admin, so its clear hides B's claim (issue #5656).
    let (_dir, store) = device_clear_scenario(GroupRole::Admin, false);
    assert!(store.tag_claims().unwrap().is_empty());
}

#[test]
fn test_a_lower_scoped_device_may_not_clear_another_authors_tag() {
    // A Write-scoped device of an Admin account has effective role Write: not Admin, so
    // its clear is retained but does not hide the claim.
    let (_dir, store) = device_clear_scenario(GroupRole::Write, false);
    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 1);
}

#[test]
fn test_a_revoked_device_may_not_clear_another_authors_tag() {
    // The grant was revoked before the clear: the device holds no active grant, so its
    // clear does not hide the claim.
    let (_dir, store) = device_clear_scenario(GroupRole::Admin, true);
    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 1);
}

#[test]
fn test_a_claim_author_may_clear_its_tag_in_the_default_view() {
    let (_dir, store, _claim, _clear) = grouped_subject_scenario('B');
    assert!(store.tag_claims().unwrap().is_empty());
}

#[test]
fn test_a_settled_group_admin_may_clear_another_authors_tag() {
    // A is the group's creator and is Admin in its settled projection; B authored the claim.
    let (_dir, store, _claim, _clear) = grouped_subject_scenario('A');
    assert!(store.tag_claims().unwrap().is_empty());
}

#[test]
fn test_an_unauthorized_clear_is_retained_but_does_not_hide_the_claim() {
    let (_dir, store, claim_id, clear_id) = grouped_subject_scenario('C');
    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, claim_id);

    // The marker remains both applied and reachable through the ordinary raw/applied APIs.
    assert!(store.event_applied(&clear_id).unwrap());
    let raw = store.event_by_id(&clear_id).unwrap().unwrap();
    assert!(matches!(
        raw.header.extensions.tag,
        Some(TagOp::Clear { claims }) if claims == vec![claim_id]
    ));
    let audit = store.applied_events(&author('C'), None, None, 10).unwrap();
    assert!(audit.iter().any(|event| event.id == clear_id));
}

#[test]
fn test_a_group_admin_has_no_clear_authority_for_ungrouped_or_missing_subjects() {
    let (_dir, mut store) = open();
    let mut chains = Chains::default();
    let group_create = chains.create_group();
    let ungrouped = chains.sign(
        'B',
        None,
        &[group_create.event.id],
        "test.subject",
        None,
        None,
    );
    let ungrouped_claim = chains.set_tag('B', ungrouped.event.id, &[ungrouped.event.id]);
    let missing_subject = Hash::digest(b"subject is not in this store");
    let missing_claim = chains.set_tag('B', missing_subject, &[ungrouped_claim.event.id]);
    let clear = chains.clear(
        'A',
        &[ungrouped_claim.event.id, missing_claim.event.id],
        &[missing_claim.event.id],
    );

    for event in [
        &group_create,
        &ungrouped,
        &ungrouped_claim,
        &missing_claim,
        &clear,
    ] {
        deliver(&mut store, event);
    }

    let claims = store.tag_claims().unwrap();
    assert_eq!(claims.len(), 2);
    assert!(
        claims
            .iter()
            .any(|claim| claim.id == ungrouped_claim.event.id)
    );
    assert!(
        claims
            .iter()
            .any(|claim| claim.id == missing_claim.event.id)
    );
}
