//! Shared-agent-worlds conformance at the public capability boundary.
//!
//! Oracle: the landed `id-caps` reference state machines. This suite checks
//! that task-board operations preserve role authority, stale shared-document
//! edits are refused and retained in audit, and human-facing visitor/presence
//! views remain legible in an agent-majority world.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_caps::agent_delegation::AgentId;
use id_caps::agent_majority::{
    Entrant, PresenceRoster, ProfileStore, Visitors, WorldProfile, starter_visitor_role,
};
use id_caps::role_config::RoleRegistry;
use id_caps::task_delegation::{Assignee, TaskDelegation, delegation_board};
use id_caps::work_surfaces::{
    ConflictModel, DocumentDefinition, DocumentPermissions, EditOp, ItemDraft, SurfaceActor,
    SurfaceError, WorkSurfaces,
};
use id_caps::world_agents::AgentPresence;
use std::collections::BTreeMap;

const WORLD: &str = "shared-worlds-conformance";
const OWNER: &str = "owner";
const T0: u64 = 1_800_000_000;
const HUMAN: [u8; 32] = [0xA0; 32];

fn agent(byte: u8) -> AgentId {
    AgentId([byte; 32])
}

fn actor(byte: u8, role: &str) -> SurfaceActor {
    SurfaceActor::agent(agent(byte), role)
}

fn registry() -> RoleRegistry {
    let mut registry = RoleRegistry::new(WORLD);
    registry.install_starters(OWNER, T0);
    registry
        .upsert(starter_visitor_role(), OWNER, T0)
        .decision
        .unwrap();
    let mut reviewer = registry.record("participant").unwrap().definition.clone();
    reviewer.name = "reviewer".to_owned();
    assert!(registry.upsert(reviewer, OWNER, T0).decision.is_ok());
    for (byte, role) in [(1, "participant"), (2, "reviewer"), (3, "participant")] {
        assert!(
            registry
                .grant(role, agent(byte), OWNER, T0)
                .decision
                .is_ok()
        );
    }
    registry
}

#[test]
fn task_board_handoff_moves_work_not_authority_or_a_side_channel() {
    let registry = registry();
    let owner = SurfaceActor::owner(OWNER);
    let mut surfaces = WorkSurfaces::new(WORLD);
    surfaces
        .define_board(&owner, delegation_board("work"), T0)
        .unwrap();
    let grants_before: Vec<_> = registry.list_grants().into_iter().cloned().collect();
    let mut delegation = TaskDelegation::new(WORLD);

    let item = delegation
        .create(
            &mut surfaces,
            &registry,
            &actor(1, "participant"),
            None,
            "work",
            ItemDraft {
                title: "review the change".to_owned(),
                ..ItemDraft::default()
            },
            Assignee::role("reviewer"),
            T0 + 1,
        )
        .unwrap();

    // Assignment is shared work state. Another agent in that role claims
    // under its own identity; the assigner cannot claim on the recipient's
    // behalf, and the role/grant set never changes.
    delegation
        .assign(
            &mut surfaces,
            &registry,
            &actor(1, "participant"),
            None,
            "work",
            item,
            Assignee::role("reviewer"),
            T0 + 2,
        )
        .unwrap();
    let board = surfaces.board(&registry, &owner, "work", T0 + 2).unwrap();
    let seen = board
        .items
        .iter()
        .find(|entry| entry.id == item)
        .unwrap()
        .version;
    assert!(
        delegation
            .claim(
                &mut surfaces,
                &registry,
                &actor(1, "participant"),
                None,
                "work",
                item,
                seen,
                T0 + 2
            )
            .is_err()
    );
    assert!(
        delegation
            .claim(
                &mut surfaces,
                &registry,
                &actor(2, "reviewer"),
                None,
                "work",
                item,
                seen,
                T0 + 3
            )
            .is_ok()
    );
    let claimed = surfaces.board(&registry, &owner, "work", T0 + 3).unwrap();
    let seen = claimed
        .items
        .iter()
        .find(|entry| entry.id == item)
        .unwrap()
        .version;
    assert!(
        delegation
            .transition(
                &mut surfaces,
                &registry,
                &actor(2, "reviewer"),
                None,
                "work",
                item,
                "doing",
                seen,
                T0 + 4,
            )
            .is_ok()
    );
    assert_eq!(
        grants_before,
        registry
            .list_grants()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    );
    let assignment = delegation.assignment("work", item).unwrap();
    assert_eq!(assignment.assignee, Assignee::role("reviewer"));
    assert_eq!(assignment.claimant, Some(actor(2, "reviewer")));
    assert!(
        delegation
            .events()
            .iter()
            .any(|event| event.outcome.starts_with("refused:"))
    );
    assert!(
        delegation
            .events()
            .iter()
            .any(|event| event.outcome == "executed")
    );
}

#[test]
fn concurrent_document_edits_refuse_stale_decisions_and_preserve_history() {
    let registry = registry();
    let owner = SurfaceActor::owner(OWNER);
    let mut surfaces = WorkSurfaces::new(WORLD);
    surfaces
        .define_document(
            &owner,
            DocumentDefinition {
                name: "notes".to_owned(),
                conflict_model: ConflictModel::LastWriterWins,
                roles: BTreeMap::from([(
                    "participant".to_owned(),
                    DocumentPermissions {
                        read: true,
                        edit: true,
                        review: true,
                    },
                )]),
            },
            T0,
        )
        .unwrap();
    surfaces
        .edit_document(
            &registry,
            &actor(1, "participant"),
            "notes",
            EditOp::Append {
                text: "first".to_owned(),
            },
            0,
            T0 + 1,
        )
        .unwrap();
    surfaces
        .edit_document(
            &registry,
            &actor(2, "participant"),
            "notes",
            EditOp::Append {
                text: "second".to_owned(),
            },
            1,
            T0 + 2,
        )
        .unwrap();
    assert_eq!(
        surfaces.edit_document(
            &registry,
            &actor(3, "participant"),
            "notes",
            EditOp::Replace {
                text: "stale overwrite".to_owned()
            },
            1,
            T0 + 3,
        ),
        Err(SurfaceError::StaleDecision { .. })
    );

    let view = surfaces
        .document(&registry, &actor(1, "participant"), "notes", T0 + 4)
        .unwrap();
    assert_eq!(view.content, "firstsecond");
    assert_eq!(view.history.len(), 2);
    assert!(
        surfaces
            .events()
            .iter()
            .any(|event| event.outcome.starts_with("refused:"))
    );
}

#[test]
fn visitor_and_presence_views_name_humans_agents_owners_and_roles() {
    let registry = registry();
    let mut profile = ProfileStore::new(WORLD, OWNER);
    profile
        .set(&registry, OWNER, WorldProfile::agent_majority(), T0)
        .unwrap();
    let mut visitors = Visitors::new(WORLD, OWNER);
    visitors
        .grant(
            &registry,
            OWNER,
            HUMAN,
            &profile.profile().visitor_role,
            T0 + 1,
        )
        .unwrap();
    visitors
        .act(
            &registry,
            &HUMAN,
            &id_caps::agent_majority::VisitorAct::Observe {
                surface: id_caps::work_surfaces::SurfaceId::Board("work".to_owned()),
            },
            T0 + 2,
        )
        .unwrap();
    assert_eq!(visitors.events().last().unwrap().outcome, "executed");

    let mut presence = PresenceRoster::new(WORLD);
    for byte in [1, 2, 3] {
        presence
            .enter(
                &registry,
                Entrant::Agent(AgentPresence {
                    agent_key: agent(byte).0,
                    agent_label: format!("worker-{byte}"),
                    owner: "team-a".to_owned(),
                    role: "participant".to_owned(),
                }),
                T0 + 3,
            )
            .unwrap();
    }
    presence
        .enter(
            &registry,
            Entrant::Human {
                key: HUMAN,
                name: "visitor".to_owned(),
                role: profile.profile().visitor_role.clone(),
            },
            T0 + 4,
        )
        .unwrap();
    let view = presence.view(&profile.profile());
    assert_eq!((view.agents, view.humans), (3, 1));
    assert_eq!(view.lines[0].name, "visitor");
    assert_eq!(
        view.lines
            .iter()
            .filter(|line| line.kind == id_caps::agent_majority::ParticipantKind::Agent)
            .count(),
        3
    );
    assert_eq!(view.agents_by_owner["team-a"].len(), 3);
    assert!(
        view.lines
            .iter()
            .filter(|line| line.owner.is_some())
            .all(|line| line.display.contains("agent of team-a"))
    );
}
