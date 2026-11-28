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

    fn append_transaction(&mut self, entries: &[JournalEntry]) -> anyhow::Result<()> {
        self.0.lock().unwrap().extend_from_slice(entries);
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

// --- Design #835 phase 2: actor-authorized operations (task 9611) ---
// The actor authorizes the live capability itself and derives the audit
// actor from it; caller-supplied actor IDs are never trusted on this path.

#[tokio::test]
async fn a_moderator_capability_revokes_and_a_guest_capability_is_denied() {
    let mut core = WorldCore::new("roles-authz", WorldLimits::default()).unwrap();
    let (victim, _victim_token) = core
        .issue_capability("victim", WorldScopes::GUEST, CapabilityBounds::default())
        .unwrap();
    let (moderator, moderator_token) = core
        .issue_capability(
            "mod",
            WorldScopes::from_role("moderator").unwrap(),
            CapabilityBounds::default(),
        )
        .unwrap();
    let (_guest, guest_token) = core
        .issue_capability("guest", WorldScopes::GUEST, CapabilityBounds::default())
        .unwrap();
    let journal = Capturing::default();
    let handle = WorldHandle::spawn_durable(
        core,
        Box::<Empty>::default(),
        Some(Box::new(journal.clone())),
    );

    // A plain guest capability carries no MODERATOR authority.
    assert!(
        handle
            .revoke_authorized(guest_token, victim.id)
            .await
            .is_err(),
        "guest capability must be denied the authorized revoke"
    );
    assert!(
        !journal.entries().iter().any(|entry| matches!(
            entry,
            JournalEntry::Revoked { participant_id, .. } if *participant_id == victim.id
        )),
        "a denied revoke must not journal a Revoked entry"
    );

    // The moderator's live capability revokes, attributed to the moderator
    // ID the actor derived from the capability — never a supplied ID.
    let revoked = handle
        .revoke_authorized(moderator_token.clone(), victim.id)
        .await
        .unwrap();
    assert!(revoked);
    let entries = journal.entries();
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::Revoked { participant_id, by: Some(by) }
                if *participant_id == victim.id && *by == moderator.id
        )),
        "expected a Revoked audit record naming the derived moderator: {entries:?}"
    );

    // A second revoke of the same participant reports no change.
    let revoked_again = handle
        .revoke_authorized(moderator_token, victim.id)
        .await
        .unwrap();
    assert!(!revoked_again);

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_builder_capability_installs_and_a_revoked_builder_is_denied() {
    let mut core = WorldCore::new("roles-authz", WorldLimits::default()).unwrap();
    let (builder, builder_token) = core
        .issue_capability(
            "builder",
            WorldScopes::from_role("builder").unwrap(),
            CapabilityBounds::default(),
        )
        .unwrap();
    let (_guest, guest_token) = core
        .issue_capability("guest", WorldScopes::GUEST, CapabilityBounds::default())
        .unwrap();
    let journal = Capturing::default();
    let handle = WorldHandle::spawn_durable(
        core,
        Box::<Empty>::default(),
        Some(Box::new(journal.clone())),
    );

    // A plain guest capability carries no BUILDER authority.
    assert!(
        handle
            .install_program_authorized(
                guest_token,
                "guest-hash".to_owned(),
                1,
                None,
                Box::<Empty>::default(),
            )
            .await
            .is_err(),
        "guest capability must be denied the authorized install"
    );
    assert!(
        !journal
            .entries()
            .iter()
            .any(|entry| matches!(entry, JournalEntry::BuilderInstalled { .. })),
        "a denied install must not journal a BuilderInstalled entry"
    );

    // The builder's live capability installs, attributed to the builder ID
    // the actor derived from the capability.
    handle
        .install_program_authorized(
            builder_token,
            "builder-hash".to_owned(),
            7,
            None,
            Box::<Empty>::default(),
        )
        .await
        .unwrap();
    let entries = journal.entries();
    assert!(
        entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::BuilderInstalled { by, module_hash, seed }
                if *by == builder.id && module_hash == "builder-hash" && *seed == 7
        )),
        "expected a BuilderInstalled audit record naming the derived builder: {entries:?}"
    );

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_revoked_moderator_capability_is_denied() {
    let mut core = WorldCore::new("roles-authz", WorldLimits::default()).unwrap();
    let (victim, _victim_token) = core
        .issue_capability("victim", WorldScopes::GUEST, CapabilityBounds::default())
        .unwrap();
    let (moderator, moderator_token) = core
        .issue_capability(
            "mod",
            WorldScopes::from_role("moderator").unwrap(),
            CapabilityBounds::default(),
        )
        .unwrap();
    let journal = Capturing::default();
    let handle = WorldHandle::spawn_durable(
        core,
        Box::<Empty>::default(),
        Some(Box::new(journal.clone())),
    );

    // The moderator is revoked by the admin path; its capability must stop
    // authorizing at the actor boundary (the check is at mutation time,
    // not at some earlier session check).
    assert!(handle.revoke(moderator.id).await.unwrap());
    assert!(
        handle
            .revoke_authorized(moderator_token, victim.id)
            .await
            .is_err(),
        "a revoked moderator capability must be denied"
    );

    handle.shutdown().await.unwrap();
}
