//! F12.3 roles v2: builder and moderator scope bits with audit.
//!
//! A `builder` participant may install modules and a `moderator` may revoke
//! participants; neither gains capability, network, file or process authority.
//! Each use of those scopes is journaled as an audit record.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use id::world::{
    CapabilityBounds, JoinCapability, JournalEntry, WorldCore, WorldHandle, WorldJournal,
    WorldLimits, WorldProgram, WorldScopes,
};

/// A capturing journal for asserting what a world records.
#[derive(Default, Clone)]
struct Capturing(Arc<Mutex<Vec<JournalEntry>>>);

impl Capturing {
    fn entries(&self) -> Vec<JournalEntry> {
        self.0.lock().unwrap().clone()
    }
}

impl WorldJournal for Capturing {
    fn append(&mut self, entry: &JournalEntry) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(entry.clone());
        Ok(())
    }
}

/// A program with no capabilities, so installs exercise only the audit path.
#[derive(Default)]
struct Empty;

impl WorldProgram for Empty {}

#[test]
fn builder_and_moderator_scopes_round_trip_roles_and_names() {
    assert_eq!(
        WorldScopes::from_role("builder").unwrap().is_builder(),
        true
    );
    assert_eq!(
        WorldScopes::from_role("moderator").unwrap().is_moderator(),
        true
    );
    assert!(!WorldScopes::from_role("guest").unwrap().is_builder());
    assert!(!WorldScopes::from_role("guest").unwrap().is_moderator());

    let scopes = WorldScopes::from_names(["join", "builder"]).unwrap();
    assert!(scopes.is_builder());
    assert!(!scopes.is_moderator());
    assert!(WorldScopes::from_names(["builder", "moderator"]).is_some());
    assert!(WorldScopes::from_names(["telepathy"]).is_none());

    // The new roles are stable protocol names.
    assert!(WorldScopes::ROLES.contains(&"builder"));
    assert!(WorldScopes::ROLES.contains(&"moderator"));
}

#[test]
fn builder_and_moderator_bits_add_no_capability_effect_authority() {
    // A builder is a guest plus the BUILDER bit; a moderator a guest plus the
    // MODERATOR bit. Neither bit widens what capabilities a participant may
    // cause: `allows_effect` must not consult them.
    let guest = WorldScopes::from_role("guest").unwrap();
    let builder = WorldScopes::from_role("builder").unwrap();
    let moderator = WorldScopes::from_role("moderator").unwrap();

    for scope in [builder, moderator] {
        assert_eq!(
            scope.allows_effect("chat.say"),
            guest.allows_effect("chat.say")
        );
        assert_eq!(
            scope.allows_effect("time.now"),
            guest.allows_effect("time.now")
        );
        assert_eq!(
            scope.allows_effect("records.read"),
            guest.allows_effect("records.read")
        );
    }

    // A scope holding only the authority bit, without JOIN or CHAT, causes
    // nothing: authority does not stand in for presence or chat.
    let builder_only = WorldScopes::from_names(["builder"]).unwrap();
    let moderator_only = WorldScopes::from_names(["moderator"]).unwrap();
    for scope in [builder_only, moderator_only] {
        assert!(!scope.allows_effect("chat.say"));
        assert!(!scope.allows_effect("time.now"));
        assert!(!scope.allows_effect("records.read"));
    }
}

#[tokio::test]
async fn a_builder_install_is_audited() {
    let core = WorldCore::new("roles", WorldLimits::default()).unwrap();
    let journal = Capturing::default();
    let handle = WorldHandle::spawn_durable(
        core,
        Box::<Empty>::default(),
        Some(Box::new(journal.clone())),
    );

    // A builder participant (id 7) installs the program.
    handle
        .install_program_by("hash".to_owned(), 1, None, Some(7), Box::<Empty>::default())
        .await
        .unwrap();

    let entries = journal.entries();
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::BuilderInstalled { by, module_hash, seed }
                if *by == 7 && module_hash == "hash" && *seed == 1
        )),
        "expected a BuilderInstalled audit record: {entries:?}"
    );

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_moderator_revoke_is_audited() {
    let mut core = WorldCore::new("roles", WorldLimits::default()).unwrap();
    let (_participant, _token): (_, JoinCapability) = core
        .issue_capability("victim", WorldScopes::GUEST, CapabilityBounds::default())
        .unwrap();
    let journal = Capturing::default();
    let handle = WorldHandle::spawn_durable(
        core,
        Box::<Empty>::default(),
        Some(Box::new(journal.clone())),
    );

    // Participant 1 is the victim; the moderator is participant 2.
    let revoked = handle.revoke_by(1, Some(2)).await.unwrap();
    assert!(revoked);

    let entries = journal.entries();
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::Revoked { participant_id, by: Some(by) }
                if *participant_id == 1 && *by == 2
        )),
        "expected a Revoked audit record naming the moderator: {entries:?}"
    );

    handle.shutdown().await.unwrap();
}
