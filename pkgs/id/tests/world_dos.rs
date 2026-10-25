//! F12.18: a consolidated denial-of-service suite.
//!
//! A guest pushes on every bound at once and the node stays up. This file
//! covers the bounds that are reachable through the public API in one place —
//! provider bounds, the `wants` document size, schedule ids, and a hostile
//! program whose every request is refused while the world keeps serving. The
//! remaining bounds have dedicated tests and are named below so this suite is
//! the map, not the only test:
//!
//! - **fuel / memory / message size**: `pkgs/id/src/sandbox.rs`
//!   (`fuel_exhaustion_traps_instead_of_hanging`, `guest_memory_growth_is_bounded_and_poisons_world`,
//!   `rejects_guest_output_outside_memory`).
//! - **journal size / frame size**: `pkgs/id/src/world_store.rs` and the
//!   session frame bounds in `pkgs/id/src/world_session/tests.rs`.
//! - **session count / open-world count**: `pkgs/id/src/world_hub.rs`
//!   (`the_session_ceiling_refuses_then_recovers`, `open_world_ceiling_refuses_when_nothing_is_evictable`).
//! - **schedule count / horizon**: `pkgs/id/src/world/tests.rs`
//!   (`schedule_bounds_refuse_each_violation_with_a_reason`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use id::world::{WorldCore, WorldHandle, WorldLimits, WorldProgram, WorldScopes};
use id::world_caps::{
    CapError, CapLedger, Decision, MAX_ARGS_BYTES, MAX_WANTS_BYTES, parse_wants,
    validate_schedule_id,
};

/// A program that asks for far more than the world grants: unknown names,
/// ungranted names, and a subscription. Every request must be refused without
/// taking the actor down.
#[derive(Default)]
struct HostileProgram;

impl WorldProgram for HostileProgram {
    fn wants(&mut self) -> anyhow::Result<Option<String>> {
        let mut requests = Vec::new();
        for i in 0..64 {
            let cap = if i % 3 == 0 {
                "telepathy" // not in the catalog: unknown_cap
            } else if i % 3 == 1 {
                "chat.say" // known, never granted: cap_denied
            } else {
                "records.read" // known, never granted: cap_denied
            };
            requests.push(format!(r#"{{"id":"r{i}","cap":"{cap}","args":{{}}}}"#));
        }
        Ok(Some(format!(
            r#"{{"v":1,"subscribe":["players"],"requests":[{}]}}"#,
            requests.join(",")
        )))
    }
}

#[test]
fn provider_bounds_refuse_with_the_documented_codes() {
    // No grants, no ceiling: every known name is refused as cap_denied.
    let mut ledger = CapLedger::new(BTreeSet::new(), false);
    assert!(matches!(
        ledger.decide("chat.say"),
        Decision::Deny(error) if error.code() == "cap_denied"
    ));

    // A name the catalog does not have is unknown_cap, even under a `*` grant.
    ledger.set_granted(BTreeSet::from(["*".to_owned()]));
    assert!(matches!(
        ledger.decide("telepathy"),
        Decision::Deny(error) if error.code() == "unknown_cap"
    ));

    // A ceiling excludes a granted name: outside_ceiling.
    let mut ledger = CapLedger::new(BTreeSet::from(["*".to_owned()]), false);
    ledger.set_allowed(BTreeSet::from(["time.now".to_owned()]));
    assert!(matches!(
        ledger.decide_for("chat.say", true),
        Decision::Deny(error) if error.code() == "outside_ceiling"
    ));

    // A participant without the scope for the effect is actor_denied.
    let mut ledger = CapLedger::new(BTreeSet::from(["*".to_owned()]), false);
    assert!(matches!(
        ledger.decide_for("chat.say", false),
        Decision::Deny(error) if error.code() == "actor_denied"
    ));

    // The remaining codes are the stable wire contract the plan documents.
    assert_eq!(CapError::bad_args().code(), "bad_args");
    assert_eq!(CapError::failed().code(), "failed");
    assert!(CapError::bad_args_with("too many").reason().is_some());
}

#[test]
fn a_wants_document_is_bounded_in_bytes_and_args() {
    // A document over MAX_WANTS_BYTES is refused by the parser.
    let oversized = format!(
        r#"{{"v":1,"requests":[{{"id":"a","cap":"time.now","args":{{"text":"{}"}}}}]}}"#,
        "x".repeat(MAX_WANTS_BYTES)
    );
    assert!(parse_wants(&oversized).is_err());

    // A request whose args exceed MAX_ARGS_BYTES is refused.
    let big_args = format!(
        r#"{{"v":1,"requests":[{{"id":"a","cap":"chat.say","args":{{"text":"{}"}}}}]}}"#,
        "y".repeat(MAX_ARGS_BYTES + 1)
    );
    assert!(parse_wants(&big_args).is_err());
}

#[test]
fn schedule_ids_are_refused_when_out_of_bounds() {
    assert!(validate_schedule_id("ok").is_ok());
    assert!(validate_schedule_id("").is_err());
    assert!(validate_schedule_id("has\nnewline").is_err());
    assert!(validate_schedule_id("caf\u{e9}").is_err());
    assert!(validate_schedule_id(&"x".repeat(65)).is_err());
}

#[tokio::test]
async fn a_hostile_program_is_refused_without_taking_the_node_down() {
    let core = WorldCore::new("dos", WorldLimits::default()).unwrap();
    let handle = WorldHandle::spawn_with_program(core, Box::new(HostileProgram));

    // The hostile wants are processed on spawn; a normal participant can still
    // join and chat, and the world still answers a capability report.
    let (_participant, token) = handle.issue("probe", WorldScopes::GUEST).await.unwrap();
    handle.chat(token.clone(), "still here").await.unwrap();

    let report = handle.caps_report().await.unwrap();
    // The unknown and ungranted requests were counted as denials, not crashes.
    let denied_total: u64 = report.usage.values().map(|usage| usage.denied).sum();
    assert!(
        denied_total >= 64,
        "expected 64 refusals, saw {denied_total}"
    );
    assert!(
        report.usage.get("telepathy").is_some_and(|u| u.denied > 0),
        "the unknown capability was refused and counted"
    );

    // A second chat still works: the world stayed responsive throughout.
    handle.chat(token, "and again").await.unwrap();
    handle.shutdown().await.unwrap();
}
