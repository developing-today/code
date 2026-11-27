//! Local scripted shared-worlds spike: three agents and one human visitor
//! exercise one task board and one shared document in the same world.
//!
//! Oracles: `id-caps` state-machine observations, and a deterministic
//! hash-chain check over the complete serialized surface, delegation,
//! visitor, and presence audit events emitted by the run. No network or
//! hosted service is used.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_caps::agent_delegation::AgentId;
use id_caps::agent_majority::{
    Entrant, PresenceRoster, ProfileStore, VisitorAct, Visitors, WorldProfile, starter_visitor_role,
};
use id_caps::agent_quota_ledger::AgentQuotaLedger;
use id_caps::mcp_policy::Quota;
use id_caps::role_config::{QuotaClass, RoleRegistry};
use id_caps::task_delegation::{Assignee, TaskDelegation, delegation_board};
use id_caps::work_surfaces::{
    ConflictModel, DocumentDefinition, DocumentPermissions, EditOp, ItemDraft, SurfaceActor,
    WorkSurfaces,
};
use id_caps::world_agents::AgentPresence;
use serde_json::Value;
use std::collections::BTreeMap;

const WORLD: &str = "shared-worlds-local-spike";
const OWNER: &str = "spike-owner";
const T0: u64 = 1_800_000_100;
const HUMAN: [u8; 32] = [0xB0; 32];

fn agent(byte: u8) -> AgentId {
    AgentId([byte; 32])
}

fn actor(byte: u8) -> SurfaceActor {
    SurfaceActor::agent(agent(byte), "participant")
}

fn hash_chain(events: &[Value]) -> Vec<[u8; 32]> {
    let mut previous = [0; 32];
    events
        .iter()
        .map(|event| {
            let bytes = serde_json::to_vec(event).unwrap();
            let mut hasher = blake3::Hasher::new();
            hasher.update(&previous);
            hasher.update(&(bytes.len() as u64).to_be_bytes());
            hasher.update(&bytes);
            previous = *hasher.finalize().as_bytes();
            previous
        })
        .collect()
}

fn verify_chain(events: &[Value], hashes: &[[u8; 32]]) -> bool {
    hashes.len() == events.len() && hash_chain(events) == hashes
}

#[test]
fn three_agents_and_a_human_complete_one_local_world_run() {
    let owner = SurfaceActor::owner(OWNER);
    let mut registry = RoleRegistry::new(WORLD);
    registry.install_starters(OWNER, T0);
    registry
        .upsert(starter_visitor_role(), OWNER, T0)
        .decision
        .unwrap();
    for byte in [1, 2, 3] {
        assert!(
            registry
                .grant("participant", agent(byte), OWNER, T0)
                .decision
                .is_ok()
        );
    }

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
    let mut presence = PresenceRoster::new(WORLD);
    for byte in [1, 2, 3] {
        presence
            .enter(
                &registry,
                Entrant::Agent(AgentPresence {
                    agent_key: agent(byte).0,
                    agent_label: format!("agent-{byte}"),
                    owner: "spike-team".to_owned(),
                    role: "participant".to_owned(),
                }),
                T0 + 2,
            )
            .unwrap();
    }
    presence
        .enter(
            &registry,
            Entrant::Human {
                key: HUMAN,
                name: "human visitor".to_owned(),
                role: profile.profile().visitor_role.clone(),
            },
            T0 + 3,
        )
        .unwrap();
    visitors
        .act(
            &registry,
            &HUMAN,
            &VisitorAct::Observe {
                surface: id_caps::work_surfaces::SurfaceId::Board("work".to_owned()),
            },
            T0 + 3,
        )
        .unwrap();

    let mut surfaces = WorkSurfaces::new(WORLD);
    surfaces
        .define_board(&owner, delegation_board("work"), T0 + 4)
        .unwrap();
    surfaces
        .define_document(
            &owner,
            DocumentDefinition {
                name: "run-notes".to_owned(),
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
            T0 + 4,
        )
        .unwrap();

    let mut tasks = TaskDelegation::new(WORLD);
    let mut ledger1 = AgentQuotaLedger::open(
        [1; 32],
        WORLD,
        "participant",
        QuotaClass {
            budget: Quota {
                frames: 0,
                tool_calls: 100,
                program_invocations: 0,
            },
            replenishment: None,
        },
        T0,
    );
    let mut ledger2 = AgentQuotaLedger::open(
        [2; 32],
        WORLD,
        "participant",
        QuotaClass {
            budget: Quota {
                frames: 0,
                tool_calls: 100,
                program_invocations: 0,
            },
            replenishment: None,
        },
        T0,
    );
    let item = tasks
        .create(
            &mut surfaces,
            &registry,
            &actor(1),
            Some(&mut ledger1),
            "work",
            ItemDraft {
                title: "prepare the release".to_owned(),
                ..ItemDraft::default()
            },
            Assignee::role("participant"),
            T0 + 5,
        )
        .unwrap();
    tasks
        .assign(
            &mut surfaces,
            &registry,
            &actor(1),
            Some(&mut ledger1),
            "work",
            item,
            Assignee::role("participant"),
            T0 + 6,
        )
        .unwrap();
    let board = surfaces.board(&registry, &owner, "work", T0 + 6).unwrap();
    let seen = board
        .items
        .iter()
        .find(|entry| entry.id == item)
        .unwrap()
        .version;
    tasks
        .claim(
            &mut surfaces,
            &registry,
            &actor(2),
            Some(&mut ledger2),
            "work",
            item,
            seen,
            T0 + 7,
        )
        .unwrap();
    let board = surfaces.board(&registry, &owner, "work", T0 + 7).unwrap();
    let seen = board
        .items
        .iter()
        .find(|entry| entry.id == item)
        .unwrap()
        .version;
    tasks
        .transition(
            &mut surfaces,
            &registry,
            &actor(2),
            Some(&mut ledger2),
            "work",
            item,
            "in-progress",
            seen,
            T0 + 8,
        )
        .unwrap();

    surfaces
        .edit_document(
            &registry,
            &actor(2),
            "run-notes",
            EditOp::Append {
                text: "initial observation;".to_owned(),
            },
            0,
            T0 + 9,
        )
        .unwrap();
    surfaces
        .edit_document(
            &registry,
            &actor(3),
            "run-notes",
            EditOp::Append {
                text: " spike complete".to_owned(),
            },
            1,
            T0 + 10,
        )
        .unwrap();

    let board = surfaces
        .board(&registry, &actor(3), "work", T0 + 11)
        .unwrap();
    assert_eq!(
        board
            .items
            .iter()
            .find(|entry| entry.id == item)
            .unwrap()
            .state,
        "in-progress"
    );
    let document = surfaces
        .document(&registry, &actor(1), "run-notes", T0 + 11)
        .unwrap();
    assert_eq!(document.content, "initial observation; spike complete");
    assert_eq!(
        (
            presence.view(&profile.profile()).agents,
            presence.view(&profile.profile()).humans
        ),
        (3, 1)
    );

    // The scripted export includes every audit stream produced by these APIs.
    let mut audit: Vec<Value> = registry
        .events()
        .iter()
        .map(|event| {
            serde_json::json!({
                "kind": event.kind,
                "actor": event.actor,
                "world": event.world,
                "role": event.role,
                "agent": event.agent,
                "outcome": event.outcome,
                "at": event.at,
            })
        })
        .collect();
    audit.extend(
        profile
            .events()
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
    );
    audit.extend(
        surfaces
            .events()
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
    );
    audit.extend(
        tasks
            .events()
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
    );
    audit.extend(
        visitors
            .events()
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
    );
    audit.extend(
        presence
            .events()
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
    );
    audit.sort_by_key(|event| event.get("at").and_then(Value::as_u64).unwrap_or_default());
    assert!(
        audit.len() >= 14,
        "full scripted audit should include configuration, presence, visitor, task, and document events"
    );
    let hashes = hash_chain(&audit);
    assert!(verify_chain(&audit, &hashes));
    let mut tampered = audit.clone();
    tampered[0]["outcome"] = Value::String("tampered".to_owned());
    assert!(!verify_chain(&tampered, &hashes));
}
