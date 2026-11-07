// Probe (lane A, 2026-10-30): does WorldCore::restore validate Issued scopes?
#![allow(clippy::unwrap_used, clippy::expect_used)]
use id::world::{
    CapabilityBounds, JoinCapability, JournalEntry, Participant, WorldCore, WorldLimits,
    WorldScopes,
};
use sha2::{Digest, Sha256};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A token whose SHA-256 digest the journal entry will carry, exactly the
/// way mint() builds one: "<id>.<hex secret>", digest = sha256(token).
fn token_for(id: u64) -> (JoinCapability, String) {
    let secret = [0x5au8; 32];
    let token = format!("{id}.{}", hex(&secret));
    let digest = hex(&Sha256::digest(token.as_bytes()));
    (JoinCapability::from_wire(token), digest)
}

fn issued(id: u64, scopes: u8, parent: Option<u64>, digest: &str) -> JournalEntry {
    JournalEntry::Issued {
        participant: Participant {
            id,
            display_name: "probe".to_owned(),
        },
        digest: digest.to_owned(),
        scopes,
        parent,
        expires_at: None,
        uses: None,
        used: 0,
        subject: None,
    }
}

fn journal(entries: Vec<JournalEntry>) -> Vec<JournalEntry> {
    let mut v = vec![JournalEntry::Created {
        world_id: "probe-world".to_owned(),
        version: 1,
    }];
    v.extend(entries);
    v
}

fn restore(entries: Vec<JournalEntry>) -> anyhow::Result<WorldCore> {
    WorldCore::restore("probe-world", WorldLimits::default(), journal(entries))
        .map(|r| r.core)
}

#[test]
fn probe_restore_scopes() {
    // CONTROL 1: a normal GUEST issue restores and its token chats.
    let (cap, digest) = token_for(1);
    let mut core = restore(vec![issued(1, WorldScopes::GUEST.bits(), None, &digest)]).unwrap();
    assert!(core.chat(&cap, "hello").is_ok(), "control guest chat");
    println!("CONTROL guest(0b00111) restore=Ok chat=Ok");

    // CONTROL 2: mint() on a live core refuses a CHAT-only capability:
    // every live path requires JOIN.
    let mut live = WorldCore::new("probe-world", WorldLimits::default()).unwrap();
    let mint_err = live.issue_capability(
        "probe",
        WorldScopes::from_bits(0b00010).unwrap(),
        CapabilityBounds::default(),
    );
    println!("CONTROL live mint CHAT-only err={:?}", mint_err.is_err());
    assert!(mint_err.is_err());

    // CONTROL 3: restore DOES validate some Issued fields (uses >= 1).
    let (_, digest3) = token_for(1);
    let bad_uses = JournalEntry::Issued {
        participant: Participant { id: 1, display_name: "p".to_owned() },
        digest: digest3,
        scopes: WorldScopes::GUEST.bits(),
        parent: None,
        expires_at: None,
        uses: Some(0),
        used: 0,
        subject: None,
    };
    let uses0_err = restore(vec![bad_uses]).is_err();
    println!("CONTROL restore uses=0 err={uses0_err}");
    assert!(uses0_err);

    // CASE A: the unnamed bit 0x80. WorldScopes::from_bits rejects it...
    assert!(WorldScopes::from_bits(0b1000_0111).is_none());
    let (_, digest_a) = token_for(1);
    let a = restore(vec![issued(1, 0b1000_0111, None, &digest_a)]);
    println!("CASE-A restore scopes=0x87 (unnamed bit) ok={}", a.is_ok());
    assert!(a.is_ok(), "restore accepted an unnamed scope bit");

    // CASE B: scopes = 0 (no JOIN, nothing) restores fine.
    let (_, digest_b) = token_for(1);
    let b = restore(vec![issued(1, 0, None, &digest_b)]);
    println!("CASE-B restore scopes=0 ok={}", b.is_ok());
    assert!(b.is_ok());

    // CASE C: CHAT-only (no JOIN) restores, and the token can CHAT even
    // though it can never snapshot/join (authorize(JOIN) fails).
    let (cap_c, digest_c) = token_for(1);
    let mut core_c = restore(vec![issued(1, 0b00010, None, &digest_c)]).unwrap();
    let chat = core_c.chat(&cap_c, "posted without joining");
    let snap = core_c.snapshot(&cap_c);
    println!("CASE-C CHAT-only chat_ok={} snapshot_ok={}", chat.is_ok(), snap.is_ok());
    assert!(chat.is_ok(), "chat-only restored capability posted chat");
    assert!(snap.is_err(), "but it cannot join/snapshot");

    // CASE D: INPUT-only (no JOIN) restores, and the token can submit input.
    let (cap_d, digest_d) = token_for(1);
    let mut core_d = restore(vec![issued(1, 0b00100, None, &digest_d)]).unwrap();
    let input = core_d.input(&cap_d, b"move");
    println!("CASE-D INPUT-only input_ok={}", input.is_ok());
    assert!(input.is_ok(), "input-only restored capability submitted input");
}
