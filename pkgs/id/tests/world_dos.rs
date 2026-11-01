//! F12.18 / F14.17: a consolidated denial-of-service suite.
//!
//! A guest pushes on every bound at once and the node stays up. Since the
//! F14.17 revision (#3840) this file drives every bound that has a real
//! refusal through the public API in one place, asserting each bound's own
//! documented refusal outcome — an F11.13 `CapError` code only where the
//! bound actually produces one:
//!
//! - **provider bounds / `wants` size / schedule ids**: `CapError` codes and
//!   parser refusals (`provider_bounds_refuse_with_the_documented_codes`,
//!   `a_wants_document_is_bounded_in_bytes_and_args`,
//!   `schedule_ids_are_refused_when_out_of_bounds`).
//! - **fuel**: the guest traps with fuel exhausted and is not poisoned
//!   (`fuel_exhaustion_traps_instead_of_hanging`).
//! - **memory**: growth past the bound fails and poisons the world; a view
//!   pointer outside linear memory is rejected
//!   (`guest_memory_growth_is_bounded_and_poisons_the_world`,
//!   `guest_output_outside_memory_is_rejected`).
//! - **frame size**: a frame over `MAX_FRAME_BYTES` is refused by the
//!   framing encoder (`a_frame_over_the_size_bound_is_refused`).
//! - **session count / open-world count**: the hub refuses with
//!   `ResolveError::Busy` / `ResolveError::TooManyWorlds`
//!   (`the_session_ceiling_refuses_with_busy_then_recovers`,
//!   `the_open_world_ceiling_refuses_with_too_many_worlds`).
//! - **schedule count / horizon**: refusals carry their reason through the
//!   handle's admin schedule path (`schedule_bounds_refuse_with_their_reasons`).
//! - **probe responsiveness**: a hostile program whose every request is
//!   refused while the world keeps serving
//!   (`a_hostile_program_is_refused_without_taking_the_node_down`).
//!
//! One bound is deliberately not driven: **journal size**. `FileJournal`
//! (`pkgs/id/src/world_store.rs`) appends with no byte cap, so there is no
//! refusal to assert; whether a cap should exist is a separate decision
//! tracked on #560. Each driven bound also keeps its dedicated in-crate
//! tests (in `sandbox.rs`, `frames.rs`, `world_hub.rs`, and
//! `world/tests.rs`); this suite is the consolidated map, not the only test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use id::frames::{MAX_FRAME_BYTES, encode_frame};
use id::sandbox::{Sandbox, SandboxLimits};
use id::world::{WorldCore, WorldHandle, WorldLimits, WorldProgram, WorldScopes};
use id::world_caps::{
    CapError, CapLedger, Decision, MAX_ARGS_BYTES, MAX_PENDING_SCHEDULES, MAX_SCHEDULE_HORIZON_MS,
    MAX_WANTS_BYTES, parse_wants, validate_schedule_id,
};
use id::world_hub::{HubLimits, OpenFuture, ResolveError, WorldHub, WorldOpener};
use id::world_session::WorldService;

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

/// Compile a WAT guest with the given sandbox limits, the way the in-crate
/// sandbox tests build theirs.
fn compile_guest(wat_source: &str, limits: SandboxLimits) -> Sandbox {
    let wasm = wat::parse_str(wat_source).unwrap();
    Sandbox::compile(&wasm, limits).unwrap()
}

#[test]
fn fuel_exhaustion_traps_instead_of_hanging() {
    // A guest that loops forever in its view: the fuel bound traps the call
    // instead of hanging the node. Fuel is the sandbox's refusal outcome —
    // a trap, not a CapError code.
    let loop_module = r#"
        (module
          (memory (export "memory") 1 1)
          (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "plaza_free") (param i32 i32))
          (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
          (func (export "plaza_update") (param i32 i32 i32 i32) (result i32) (i32.const 0))
          (func (export "plaza_view") (param i32) (result i32) (i32.const 0)
            (loop $again (br $again)))
          (func (export "plaza_out_len") (result i32) (i32.const 0)))
    "#;
    let runner = compile_guest(
        loop_module,
        SandboxLimits {
            fuel: 100,
            ..SandboxLimits::default()
        },
    );
    let mut world = runner.instantiate(0).unwrap();
    assert!(
        world
            .view()
            .unwrap_err()
            .to_string()
            .contains("fuel exhausted")
    );
    // A trapped view does not poison the instance (F11.17): the retry runs
    // the same view and exhausts fuel again, rather than being refused up
    // front as poisoned.
    assert!(
        world
            .view()
            .unwrap_err()
            .to_string()
            .contains("fuel exhausted")
    );
}

#[test]
fn guest_memory_growth_is_bounded_and_poisons_the_world() {
    // A guest that grows memory on every update: growth past the bound
    // fails, and the failure poisons the world — the documented memory
    // refusal outcome.
    let memory_module = r#"
        (module
          (memory (export "memory") 1)
          (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "plaza_free") (param i32 i32))
          (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
          (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
            i32.const 1 memory.grow drop
            local.get 0)
          (func (export "plaza_view") (param i32) (result i32) (i32.const 0))
          (func (export "plaza_out_len") (result i32) (i32.const 0)))
    "#;
    let mut world = compile_guest(
        memory_module,
        SandboxLimits {
            memory_bytes: 64 * 1024,
            ..SandboxLimits::default()
        },
    )
    .instantiate(0)
    .unwrap();
    let error = world.update(1, b"event").unwrap_err().to_string();
    assert!(error.contains("memory.grow") || error.contains("failed"));
    assert!(world.view().unwrap_err().to_string().contains("poisoned"));
}

#[test]
fn guest_output_outside_memory_is_rejected() {
    // A guest whose view points outside its linear memory (one 64KiB page,
    // view pointer at the first byte past it): the host refuses the output
    // instead of reading out of bounds.
    let outside_module = r#"
        (module
          (memory (export "memory") 1 1)
          (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "plaza_free") (param i32 i32))
          (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
          (func (export "plaza_update") (param i32 i32 i32 i32) (result i32) (i32.const 0))
          (func (export "plaza_view") (param i32) (result i32) (i32.const 65536))
          (func (export "plaza_out_len") (result i32) (i32.const 1)))
    "#;
    let runner = compile_guest(outside_module, SandboxLimits::default());
    assert!(
        runner
            .instantiate(0)
            .unwrap()
            .view()
            .unwrap_err()
            .to_string()
            .contains("outside linear memory")
    );
}

#[test]
fn a_frame_over_the_size_bound_is_refused() {
    // The framing layer's refusal is a generic framing error naming the
    // bound, not a CapError code: every writer goes through encode_frame.
    let oversized = vec![0_u8; MAX_FRAME_BYTES + 1];
    let error = encode_frame(&oversized).unwrap_err().to_string();
    assert!(error.contains("exceeds"), "{error}");
    // A frame at exactly the bound is still framed.
    let at_bound = vec![0_u8; MAX_FRAME_BYTES];
    assert_eq!(encode_frame(&at_bound).unwrap().len(), MAX_FRAME_BYTES + 4);
}

/// An in-memory [`WorldOpener`] for the hub bounds, mirroring the hub's
/// in-crate test opener: ephemeral worlds that can never be evicted.
#[derive(Default)]
struct MemoryOpener {
    known: StdMutex<Vec<String>>,
}

impl WorldOpener for MemoryOpener {
    fn open<'a>(&'a self, name: &'a str, create: bool) -> OpenFuture<'a> {
        Box::pin(async move {
            let mut known = self.known.lock().unwrap();
            if !known.iter().any(|n| n == name) {
                if !create {
                    return Ok(None);
                }
                known.push(name.to_owned());
            }
            let world = WorldHandle::spawn(WorldCore::new(name, WorldLimits::default())?);
            Ok(Some(WorldService::new(world, Some("admin".to_owned()))))
        })
    }

    fn exists(&self, name: &str) -> bool {
        self.known.lock().unwrap().iter().any(|n| n == name)
    }

    fn list(&self) -> Vec<String> {
        self.known.lock().unwrap().clone()
    }

    fn durable(&self) -> bool {
        false
    }
}

fn dos_hub(limits: HubLimits) -> WorldHub {
    let opener: Arc<dyn WorldOpener> = Arc::new(MemoryOpener::default());
    WorldHub::new(opener, Some("admin".to_owned()), "lobby", limits)
}

#[tokio::test]
async fn the_session_ceiling_refuses_with_busy_then_recovers() {
    let hub = dos_hub(HubLimits {
        max_sessions: 2,
        ..HubLimits::default()
    });
    let first = hub.lease(Some("a"), Some("admin")).await.unwrap();
    let _second = hub.lease(Some("a"), Some("admin")).await.unwrap();
    // The session ceiling's documented refusal is ResolveError::Busy.
    assert_eq!(
        hub.lease(Some("a"), Some("admin")).await.err(),
        Some(ResolveError::Busy)
    );
    // Dropping a lease frees a session: the hub recovers.
    drop(first);
    hub.lease(Some("a"), Some("admin")).await.unwrap();
    hub.shutdown_all().await;
}

#[tokio::test]
async fn the_open_world_ceiling_refuses_with_too_many_worlds() {
    let hub = dos_hub(HubLimits {
        max_open_worlds: 2,
        ..HubLimits::default()
    });
    let _a = hub.lease(Some("a"), Some("admin")).await.unwrap();
    let _b = hub.lease(Some("b"), Some("admin")).await.unwrap();
    // Ephemeral worlds can never be evicted, so a third world is refused
    // with ResolveError::TooManyWorlds.
    assert_eq!(
        hub.lease(Some("c"), Some("admin")).await.err(),
        Some(ResolveError::TooManyWorlds)
    );
    // Existing worlds are still reachable: the refusal takes nothing down.
    hub.lease(Some("a"), None).await.unwrap();
    hub.shutdown_all().await;
}

#[tokio::test]
async fn schedule_bounds_refuse_with_their_reasons() {
    let core = WorldCore::new("dos-schedules", WorldLimits::default()).unwrap();
    let handle = WorldHandle::spawn(core);
    let (probe, probe_token) = handle.issue("probe", WorldScopes::GUEST).await.unwrap();
    let _ = probe;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    // Horizon: a schedule past MAX_SCHEDULE_HORIZON_MS is refused, and the
    // refusal says why (the program itself sees only bad_args).
    let error = handle
        .admin_schedule_at("too-far".to_owned(), now_ms + MAX_SCHEDULE_HORIZON_MS + 1)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("days past"), "{error}");

    // The admin path replaces a pending id in place (it validates as a
    // replacement, #807); the duplicate refusal itself is the program
    // path's, covered by the dedicated schedule-bounds test in
    // `world/tests.rs`. Here the replacement must show the new time and
    // still count as the one pending schedule.
    handle
        .admin_schedule_at("taken".to_owned(), now_ms + 5_000)
        .await
        .unwrap();
    let pending = handle
        .admin_schedule_at("taken".to_owned(), now_ms + 9_000)
        .await
        .unwrap();
    assert_eq!(pending.get("taken"), Some(&(now_ms + 9_000)));
    assert_eq!(pending.len(), 1);

    // Count: with MAX_PENDING_SCHEDULES pending, one more is refused.
    for index in 0..MAX_PENDING_SCHEDULES - 1 {
        handle
            .admin_schedule_at(format!("s{index}"), now_ms + 60_000)
            .await
            .unwrap();
    }
    let error = handle
        .admin_schedule_at("one-more".to_owned(), now_ms + 60_000)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("pending schedules"), "{error}");

    // Throughout, the probe participant can still chat: the refusals never
    // took the world down.
    handle.chat(probe_token, "still here").await.unwrap();
    handle.shutdown().await.unwrap();
}
