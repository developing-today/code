//! Probe (lane A, 2026-10-30): does WorldCore::restore validate the
//! Issued arm's participant id and display_name the way mint does?
//! - mint never issues participant id 0 (the host/system identity;
//!   next_participant_id starts at 1) and ids it commits as are the
//!   host's own (system_chat commits participant_id 0).
//! - mint sanitizes the display name and refuses one that sanitizes
//!   to empty (core.rs mint: sanitize_display_name + non-empty check).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use id::world::{
    CapabilityBounds, JoinCapability, JournalEntry, Participant, WorldCore, WorldLimits,
    WorldScopes, JOURNAL_VERSION,
};

const SECRET_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

fn token_for(id: u64) -> JoinCapability {
    JoinCapability::from_wire(format!("{id}.{SECRET_HEX}"))
}

/// SHA-256 hex of the token string, computed with python3 hashlib and
/// hardcoded (token_digest is pub(crate)): sha256("<id>.<secret hex>").
fn digest_for(id: u64) -> String {
    match id {
        0 => "7fd7b235a2ff3b64428f655562d7d181e169cc72051e1d34eb700a3e47067222".to_owned(),
        1 => "a738f3f62ef0bbdcb5f23580e7cae07cdb1bd280fb15210941393b7f7be6d02d".to_owned(),
        2 => "1a097938e88d4622c6a14b752923ae2033235f332dec914f69ecb5a7663d74c0".to_owned(),
        _ => unreachable!(),
    }
}

fn issued(id: u64, name: &str) -> JournalEntry {
    JournalEntry::Issued {
        participant: Participant {
            id,
            display_name: name.to_owned(),
        },
        digest: digest_for(id),
        scopes: WorldScopes::GUEST.bits(),
        parent: None,
        expires_at: None,
        uses: None,
        used: 0,
        subject: None,
    }
}

fn created() -> JournalEntry {
    JournalEntry::Created {
        world_id: "w".to_owned(),
        version: JOURNAL_VERSION,
    }
}

#[test]
fn probe_controls_and_cases() {
    // CONTROL live: the first capability mint issues is id 1, never 0.
    let mut live = WorldCore::new("w", WorldLimits::default()).unwrap();
    let (first, _t) = live
        .issue_capability("Ada", WorldScopes::GUEST, CapabilityBounds::default())
        .unwrap();
    println!("CONTROL live first issued participant id = {}", first.id);
    assert_eq!(first.id, 1);

    // CONTROL live: mint refuses a name that sanitizes to empty, and
    // strips control characters from a name it accepts.
    let mut live2 = WorldCore::new("w", WorldLimits::default()).unwrap();
    let refused = live2.issue_capability(
        "\u{7f}\u{1b}",
        WorldScopes::GUEST,
        CapabilityBounds::default(),
    );
    println!("CONTROL live mint control-only name refused = {}", refused.is_err());
    assert!(refused.is_err());
    let (stripped, _t2) = live2
        .issue_capability("a\u{1b}b\nc", WorldScopes::GUEST, CapabilityBounds::default())
        .unwrap();
    println!("CONTROL live mint sanitizes name to {:?}", stripped.display_name);
    assert_eq!(stripped.display_name, "abc");

    // CONTROL restore: a normal Issued participant 1 restores and chats as 1.
    let mut restored = WorldCore::restore(
        "w",
        WorldLimits::default(),
        [created(), issued(1, "Ada")],
    )
    .unwrap();
    let event = restored.core.chat(&token_for(1), "hello from one").unwrap();
    println!("CONTROL restore participant 1 chat participant_id = {}", event.participant_id);
    assert_eq!(event.participant_id, 1);

    // CASE A: Issued participant 0 — the host identity.
    let restore0 = WorldCore::restore(
        "w",
        WorldLimits::default(),
        [created(), issued(0, "host-impostor")],
    );
    println!("CASE A restore Issued(id=0) ok = {}", restore0.is_ok());
    if let Ok(mut restored0) = restore0 {
        let snap = restored0.core.snapshot(&token_for(0)).unwrap();
        println!(
            "CASE A snapshot via token 0 ok; participants = {:?}",
            snap.participants
                .iter()
                .map(|p| (p.id, p.display_name.clone()))
                .collect::<Vec<_>>()
        );
        let event = restored0.core.chat(&token_for(0), "trust me, the host").unwrap();
        println!(
            "CASE A chat via token 0 committed with participant_id = {} (0 is the host/system id)",
            event.participant_id
        );
    }

    // CASE B: Issued participant 2 whose display name is all control
    // characters (sanitizes to empty live) / contains a raw newline.
    let restore2 = WorldCore::restore(
        "w",
        WorldLimits::default(),
        [created(), issued(2, "\u{7f}\u{1b}")],
    );
    println!("CASE B restore Issued(name=all-controls) ok = {}", restore2.is_ok());
    if let Ok(restored2) = restore2 {
        let snap = restored2.core.snapshot(&token_for(2)).unwrap();
        println!(
            "CASE B snapshot participants = {:?}",
            snap.participants
                .iter()
                .map(|p| (p.id, p.display_name.clone()))
                .collect::<Vec<_>>()
        );
    }
    let restore2b = WorldCore::restore(
        "w",
        WorldLimits::default(),
        [created(), issued(2, "ada\n[system] the host has left")],
    );
    println!("CASE B2 restore Issued(name with newline) ok = {}", restore2b.is_ok());
    if let Ok(restored2b) = restore2b {
        let snap = restored2b.core.snapshot(&token_for(2)).unwrap();
        println!(
            "CASE B2 snapshot participants = {:?}",
            snap.participants
                .iter()
                .map(|p| (p.id, p.display_name.clone()))
                .collect::<Vec<_>>()
        );
    }
}
