// Probe (lane A, 2026-10-31): does WorldCore::restore enforce the
// participant limit and display-name sanitization that mint enforces?
use id::world::{
    CapabilityBounds, JOURNAL_VERSION, JoinCapability, JournalEntry, Participant, WorldCore,
    WorldLimits, WorldScopes,
};
use sha2::{Digest, Sha256};

fn secret(id: u64) -> String {
    format!("{:02x}", id).repeat(32)
}
fn token(id: u64) -> String {
    format!("{id}.{}", secret(id))
}
fn digest(id: u64) -> String {
    let hash = Sha256::digest(token(id).as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}
fn issued(id: u64, name: &str) -> JournalEntry {
    JournalEntry::Issued {
        participant: Participant { id, display_name: name.to_owned() },
        digest: digest(id),
        scopes: WorldScopes::GUEST.bits(),
        parent: None,
        expires_at: None,
        uses: None,
        used: 0,
        subject: None,
    }
}
fn header() -> JournalEntry {
    JournalEntry::Created { world_id: "test-world".to_owned(), version: JOURNAL_VERSION }
}
fn limits(n: usize) -> WorldLimits {
    WorldLimits { participants: n, ..WorldLimits::default() }
}

#[test]
fn probe_restore_limits_and_names() {
    // CONTROL (live mint): limit 2 admits two, refuses the third.
    let mut live = WorldCore::new("test-world", limits(2)).unwrap();
    assert!(live.issue_capability("A", WorldScopes::GUEST, CapabilityBounds::default()).is_ok());
    assert!(live.issue_capability("B", WorldScopes::GUEST, CapabilityBounds::default()).is_ok());
    let mint_third = live.issue_capability("C", WorldScopes::GUEST, CapabilityBounds::default());
    println!("CONTROL mint third over limit: is_err={}", mint_third.is_err());
    assert!(mint_third.is_err());

    // CONTROL (restore at limit): two Issued under limit 2 restores Ok.
    let at_limit = WorldCore::restore("test-world", limits(2), vec![header(), issued(1, "A"), issued(2, "B")]);
    println!("CONTROL restore at limit: is_ok={}", at_limit.is_ok());
    assert!(at_limit.is_ok());

    // CASE A: three Issued under limit 2.
    let over = WorldCore::restore(
        "test-world",
        limits(2),
        vec![header(), issued(1, "A"), issued(2, "B"), issued(3, "C")],
    );
    println!("CASE A restore 3 issued under limit 2: is_ok={}", over.is_ok());
    if let Ok(restored) = over {
        let core = restored.core;
        let excess = JoinCapability::from_wire(token(3));
        let snap = core.snapshot(&excess);
        println!(
            "CASE A excess participant 3 snapshot: is_ok={} participants_in_snapshot={}",
            snap.is_ok(),
            snap.as_ref().map(|s| s.participants.len()).unwrap_or(0)
        );
        let mut core = core;
        let chat = core.chat(&excess, "hello from over-limit participant");
        println!("CASE A excess participant 3 chat: is_ok={}", chat.is_ok());
    }

    // CONTROL (mint name): empty-after-sanitize name is refused live.
    let mut live2 = WorldCore::new("test-world", limits(4)).unwrap();
    let mint_empty = live2.issue_capability("\u{1}\u{2}", WorldScopes::GUEST, CapabilityBounds::default());
    println!("CONTROL mint control-only name: is_err={}", mint_empty.is_err());
    assert!(mint_empty.is_err());

    // CASE B: Issued with an empty display name.
    let empty = WorldCore::restore("test-world", limits(4), vec![header(), issued(1, "")]);
    println!("CASE B restore empty display_name: is_ok={}", empty.is_ok());
    if let Ok(restored) = empty {
        let t = JoinCapability::from_wire(token(1));
        let snap = restored.core.snapshot(&t).unwrap();
        println!("CASE B snapshot participant names: {:?}", snap.participants.iter().map(|p| p.display_name.clone()).collect::<Vec<_>>());
    }

    // CASE C: Issued with control chars + a 100-char name (mint sanitizes:
    // strips controls, caps at 48 chars).
    let long_name = format!("a\u{7}\n{}", "x".repeat(98));
    let dirty = WorldCore::restore("test-world", limits(4), vec![header(), issued(1, &long_name)]);
    println!("CASE C restore dirty/long display_name: is_ok={}", dirty.is_ok());
    if let Ok(restored) = dirty {
        let t = JoinCapability::from_wire(token(1));
        let snap = restored.core.snapshot(&t).unwrap();
        let name = &snap.participants[0].display_name;
        println!("CASE C restored name verbatim={} len={} has_control={}", name == &long_name, name.chars().count(), name.chars().any(|c| c.is_control()));
    }

    // CASE D (boundary for triage): Issued x3, Revoked(2), Issued(4) under
    // limit 2 — active count never exceeds 2 if revokes count in order.
    let with_revoke = WorldCore::restore(
        "test-world",
        limits(2),
        vec![
            header(),
            issued(1, "A"),
            issued(2, "B"),
            JournalEntry::Revoked { participant_id: 2, by: None },
            issued(3, "C"),
        ],
    );
    println!("CASE D restore issue/revoke/issue at limit 2: is_ok={}", with_revoke.is_ok());
}
