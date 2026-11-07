// Probe (lane A, 2026-11-01): does WorldCore::restore accept an Issued
// delegation whose parent was already revoked earlier in the journal?
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
    WorldCore::restore("probe-world", WorldLimits::default(), journal(entries)).map(|r| r.core)
}

#[test]
fn probe_restore_delegate_of_revoked_parent() {
    let parent_scopes = WorldScopes::GUEST.bits() | WorldScopes::DELEGATE.bits();

    // CONTROL A: parent and child both issued, THEN the parent is revoked.
    // revoke_tree takes the child with it; the child's token must fail.
    let (cap_a, digest_a) = token_for(2);
    let (_, digest_pa) = token_for(1);
    let mut core_a = restore(vec![
        issued(1, parent_scopes, None, &digest_pa),
        issued(2, WorldScopes::GUEST.bits(), Some(1), &digest_a),
        JournalEntry::Revoked {
            participant_id: 1,
            by: None,
        },
    ])
    .unwrap();
    let a_chat = core_a.chat(&cap_a, "hello");
    println!("CONTROL A revoke-after-issue: chat err={}", a_chat.is_err());
    assert!(a_chat.is_err(), "control A: revoked tree must not act");

    // CONTROL B: the live path refuses to delegate from a revoked parent.
    let mut live = WorldCore::new("probe-world", WorldLimits::default()).unwrap();
    let (_, parent_token) = live
        .issue_capability(
            "parent",
            WorldScopes::from_bits(parent_scopes).unwrap(),
            CapabilityBounds::default(),
        )
        .unwrap();
    live.revoke_tree(1);
    let live_delegate = live.attenuate(
        &parent_token,
        WorldScopes::GUEST,
        CapabilityBounds::default(),
        "child",
    );
    println!(
        "CONTROL B live delegate-from-revoked err={}",
        live_delegate.is_err()
    );
    assert!(live_delegate.is_err(), "control B: live delegation must fail");

    // CASE: the parent is revoked BEFORE the child's Issued entry appears.
    // Restore's Issued arm checks only that the parent exists; the Revoked
    // arm's revoke_tree ran when the child did not exist yet.
    let (cap_c, digest_c) = token_for(2);
    let (_, digest_pc) = token_for(1);
    let restored = restore(vec![
        issued(1, parent_scopes, None, &digest_pc),
        JournalEntry::Revoked {
            participant_id: 1,
            by: None,
        },
        issued(2, WorldScopes::GUEST.bits(), Some(1), &digest_c),
    ]);
    match restored {
        Ok(mut core) => {
            let chat = core.chat(&cap_c, "hello from a revoked line");
            println!(
                "CASE restore=Ok child-of-revoked-parent chat ok={}",
                chat.is_ok()
            );
        }
        Err(error) => {
            println!("CASE restore=Err {error:#}");
        }
    }
}
