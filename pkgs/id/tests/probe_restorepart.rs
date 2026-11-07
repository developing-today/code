use id::world::{
    JOURNAL_VERSION, JournalEntry, Participant, WorldCore, WorldEvent, WorldEventKind,
    WorldLimits, WorldScopes,
};
use sha2::{Digest, Sha256};

fn digest_of(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn header() -> JournalEntry {
    JournalEntry::Created {
        world_id: "w".to_owned(),
        version: JOURNAL_VERSION,
    }
}

fn issued(id: u64) -> JournalEntry {
    JournalEntry::Issued {
        participant: Participant {
            id,
            display_name: format!("p{id}"),
        },
        digest: digest_of(&format!("{id}.{}", "ab".repeat(32))),
        scopes: WorldScopes::GUEST.bits(),
        parent: None,
        expires_at: None,
        uses: None,
        used: 0,
        subject: None,
    }
}

fn committed(seq: u64, participant_id: u64, kind: WorldEventKind) -> JournalEntry {
    JournalEntry::Committed {
        event: WorldEvent {
            sequence: seq,
            participant_id,
            kind,
        },
    }
}

#[test]
fn probe_restore_committed_participants() {
    // CONTROL 1: committed input from an issued participant restores and replays.
    let r = WorldCore::restore(
        "w",
        WorldLimits::default(),
        vec![
            header(),
            issued(1),
            committed(1, 1, WorldEventKind::Input(b"hi".to_vec())),
        ],
    );
    println!("CONTROL issued-input: ok={} replay={:?}", r.is_ok(), r.as_ref().map(|x| x.replay.len()));
    assert!(r.is_ok());

    // CONTROL 2: committed chat from participant 0 (the host) restores.
    let r = WorldCore::restore(
        "w",
        WorldLimits::default(),
        vec![
            header(),
            committed(1, 0, WorldEventKind::Chat("host line".to_owned())),
        ],
    );
    println!("CONTROL host-chat: ok={}", r.is_ok());
    assert!(r.is_ok());

    // CASE A: committed input from participant 999, never issued.
    let r = WorldCore::restore(
        "w",
        WorldLimits::default(),
        vec![
            header(),
            issued(1),
            committed(1, 999, WorldEventKind::Input(b" forged".to_vec())),
        ],
    );
    match &r {
        Ok(restored) => println!(
            "CASE A ghost-input: RESTORED Ok; replay_len={} replay_participants={:?} snapshot_participants={:?}",
            restored.replay.len(),
            restored.replay.iter().map(|e| e.participant_id).collect::<Vec<_>>(),
            restored.core.snapshot(&id::world::JoinCapability::from_wire(format!("1.{}", "ab".repeat(32)))).unwrap().participants.iter().map(|p| p.id).collect::<Vec<_>>(),
        ),
        Err(e) => println!("CASE A ghost-input: refused: {e}"),
    }

    // CASE B: committed chat from participant 999 is retained and shown in a legit snapshot.
    let r = WorldCore::restore(
        "w",
        WorldLimits::default(),
        vec![
            header(),
            issued(1),
            committed(1, 999, WorldEventKind::Chat("ghost says hi".to_owned())),
        ],
    );
    match &r {
        Ok(restored) => {
            let token = id::world::JoinCapability::from_wire(format!("1.{}", "ab".repeat(32)));
            let snap = restored.core.snapshot(&token).unwrap();
            println!(
                "CASE B ghost-chat: RESTORED Ok; snapshot participants={:?} events={:?}",
                snap.participants.iter().map(|p| p.id).collect::<Vec<_>>(),
                snap.events
                    .iter()
                    .map(|e| (e.participant_id, format!("{:?}", e.kind)))
                    .collect::<Vec<_>>(),
            );
        }
        Err(e) => println!("CASE B ghost-chat: refused: {e}"),
    }

    // CASE C: checkpoint events from participant 999.
    let r = WorldCore::restore(
        "w",
        WorldLimits::default(),
        vec![
            header(),
            issued(1),
            JournalEntry::Checkpoint {
                sequence: 1,
                module_hash: "m".to_owned(),
                seed: 0,
                provenance: None,
                execution_profile: None,
                caps_ceiling: None,
                schedules: None,
                snapshot: String::new(),
                events: vec![WorldEvent {
                    sequence: 1,
                    participant_id: 999,
                    kind: WorldEventKind::Input(b"x".to_vec()),
                }],
            },
        ],
    );
    match &r {
        Ok(restored) => println!(
            "CASE C checkpoint-ghost: RESTORED Ok; sequence={}",
            restored.core.current_sequence()
        ),
        Err(e) => println!("CASE C checkpoint-ghost: refused: {e}"),
    }
}
