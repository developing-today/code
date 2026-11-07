use id::groups::event::{EventBody, GroupEvent, decode, encode_group_event};
use id::groups::{Body, Create, Policy, Role, Template};
use p2panda_core::Hash;

fn base(body: EventBody) -> GroupEvent {
    GroupEvent {
        id: Hash::from_bytes([1; 32]),
        author: [2; 32],
        after: vec![Hash::from_bytes([4; 32])],
        group: Some(Hash::from_bytes([3; 32])),
        body,
    }
}

#[test]
fn probe_groupevent_codec_gaps() {
    // control: an ordinary join round-trips
    let ok = base(EventBody::Group(Body::Join));
    let bytes = encode_group_event(&ok).expect("control encodes");
    assert_eq!(decode(&bytes).expect("control decodes"), ok);
    println!("CONTROL join round-trip: ok");

    // 1: duplicate causal predecessors
    let mut dup_after = base(EventBody::Group(Body::Join));
    dup_after.after = vec![Hash::from_bytes([4; 32]), Hash::from_bytes([4; 32])];
    match encode_group_event(&dup_after) {
        Ok(bytes) => println!(
            "DUP_AFTER encode=Ok decode={:?}",
            decode(&bytes).map(|d| d.after.len())
        ),
        Err(e) => println!("DUP_AFTER encode=Err {e}"),
    }

    // 2: the event names ITSELF as a causal predecessor (unplaceable cycle)
    let mut self_after = base(EventBody::Group(Body::Join));
    self_after.after = vec![self_after.id];
    match encode_group_event(&self_after) {
        Ok(bytes) => println!(
            "SELF_AFTER encode=Ok decode={:?}",
            decode(&bytes).map(|d| d.after.len())
        ),
        Err(e) => println!("SELF_AFTER encode=Err {e}"),
    }

    // 3: duplicate create member with conflicting roles
    let dup_members = base(EventBody::Group(Body::Create(Create {
        template: Template::Custom,
        policy: Policy::default(),
        members: vec![([6; 32], Role::Read), ([6; 32], Role::Admin)],
        admins_on_create: false,
    })));
    match encode_group_event(&dup_members) {
        Ok(bytes) => println!(
            "DUP_MEMBER encode=Ok decode={:?}",
            decode(&bytes).map(|d| match d.body {
                EventBody::Group(Body::Create(c)) => c.members.len(),
                _ => 999,
            })
        ),
        Err(e) => println!("DUP_MEMBER encode=Err {e}"),
    }
}
