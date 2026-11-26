//! Hosted/local parity conformance suite (#10698, packet P6 of the
//! hosted-worlds v1 design `doc/2026-11-16_design_id6283_hosted_worlds_v1.md`
//! §7 and §9).
//!
//! One journal fixture is copied into a local world directory and into a
//! host data root. The local side opens it the way `id serve --world`
//! does, straight through [`open_world`]; the hosted side starts it through
//! the [`Supervisor`]. Both must give the same replay report, the same
//! snapshot bytes, the same directory view, the same policy, the same
//! membership outcomes, the same lifecycle verdicts and the same module
//! addresses, and neither may rewrite the journal. A host-only setting
//! (a pin and operator-only wake) must change none of it.

use std::fmt::Write as _;
use std::path::Path;

use super::*;
use crate::directory_auth::Caller;
use crate::directory_view::DirectoryAction;
use crate::world::{Authority, Isolation, JoinCapability, WorldScopes};
use tempfile::TempDir;

const WORLD: &str = "parity";

/// The fixture's cast, as the world knows them.
struct Cast {
    owner: String,
    guest: String,
    stranger: String,
    ann: JoinCapability,
    revoked: JoinCapability,
}

fn key_hex(seed: u8) -> String {
    SigningKey::from_bytes(&[seed; 32])
        .verifying_key()
        .as_bytes()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

fn keyed(key: &str) -> Caller {
    Caller {
        key: Some(key.to_owned()),
        ..Caller::default()
    }
}

async fn open_local(dir: &Path) -> (WorldService, OpenReport) {
    open_world(
        dir,
        WORLD,
        WorldLimits::default(),
        RuntimeLimits::default(),
        |handle: WorldHandle| WorldService::new(handle, None),
    )
    .await
    .unwrap()
}

/// Write the fixture journal: an owner with a group, a guest, members
/// who chat, one revoked, and a policy change.
async fn write_fixture(dir: &Path) -> Cast {
    let (service, _) = open_local(dir).await;
    let cast = Cast {
        owner: key_hex(11),
        guest: key_hex(12),
        stranger: key_hex(13),
        ann: JoinCapability::from_wire(String::new()),
        revoked: JoinCapability::from_wire(String::new()),
    };
    for (key, name) in [(&cast.owner, "Owner"), (&cast.guest, "Guest")] {
        service
            .directory(keyed(key), DirectoryAction::SignUpKey { name: name.into() })
            .await
            .unwrap();
    }
    service
        .directory(
            keyed(&cast.owner),
            DirectoryAction::CreateGroup {
                name: "keepers".into(),
                description: "the people who keep this world".into(),
                scopes: Vec::new(),
            },
        )
        .await
        .unwrap();
    let world = service.world();
    let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
    let (bo, revoked) = world.issue("bo", WorldScopes::GUEST).await.unwrap();
    for line in ["hello", "is anyone here", "bo, you there?"] {
        world.chat(ann.clone(), line).await.unwrap();
    }
    world.chat(revoked.clone(), "here, briefly").await.unwrap();
    assert!(world.revoke(bo.id).await.unwrap());
    world
        .set_isolation(Authority::Admin, Isolation::Unisolated)
        .await
        .unwrap();
    world.shutdown().await.unwrap();
    Cast {
        ann,
        revoked,
        ..cast
    }
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Every world-visible result one side gives for the fixture.
#[derive(Debug, PartialEq, Eq)]
struct Observation {
    sequence: u64,
    journal_entries: usize,
    module_hash: Option<String>,
    replayed_inputs: usize,
    program_error: Option<String>,
    snapshot: Vec<u8>,
    directory: Vec<u8>,
    policy: Vec<u8>,
    ann_chats: bool,
    revoked_refused: bool,
    stranger_refused: bool,
    modules: Vec<String>,
}

async fn observe(
    service: &WorldService,
    report: &OpenReport,
    dir: &Path,
    cast: &Cast,
) -> Observation {
    let world = service.world();
    let snapshot = serde_json::to_vec(&world.snapshot(cast.ann.clone()).await.unwrap()).unwrap();
    let directory = serde_json::to_vec(
        &service
            .directory(keyed(&cast.owner), DirectoryAction::View)
            .await
            .unwrap()
            .view,
    )
    .unwrap();
    let policy = serde_json::to_vec(&world.policy().await.unwrap()).unwrap();
    let ann_chats = world.participant_id(cast.ann.clone()).await.is_ok();
    let revoked_refused = world.participant_id(cast.revoked.clone()).await.is_err();
    let stranger_refused = world
        .participant_id(JoinCapability::from_wire("not-a-member".into()))
        .await
        .is_err();
    let mut modules: Vec<String> = std::fs::read_dir(dir.join("modules"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    modules.sort();
    Observation {
        sequence: report.sequence,
        journal_entries: report.journal_entries,
        module_hash: report.module_hash.clone(),
        replayed_inputs: report.replayed_inputs,
        program_error: report.program_error.clone(),
        snapshot,
        directory,
        policy,
        ann_chats,
        revoked_refused,
        stranger_refused,
        modules,
    }
}

fn authorities(cast: &Cast) -> Vec<(&'static str, Authority)> {
    vec![
        ("operator", Authority::Admin),
        ("owner", Authority::Signed(cast.owner.clone())),
        ("guest", Authority::Signed(cast.guest.clone())),
        ("stranger", Authority::Signed(cast.stranger.clone())),
    ]
}

/// The verdict a local host gives: the world's own answer, and restore
/// for the operator alone.
async fn local_verdict(
    service: &WorldService,
    act: LifecycleAct,
    authority: &Authority,
) -> Result<(), String> {
    if act == LifecycleAct::Restore && *authority != Authority::Admin {
        return Err(restore_refusal().to_string());
    }
    service
        .world()
        .authorize_lifecycle(authority.clone(), act.phrase())
        .await
        .map_err(|error| error.to_string())
}

const ACTS: [LifecycleAct; 4] = [
    LifecycleAct::Wake,
    LifecycleAct::Pause,
    LifecycleAct::Archive,
    LifecycleAct::Restore,
];

/// Start the fixture on a host root; `backend` sets host-only knobs first.
async fn hosted(root: &Path, backend: bool) -> Supervisor {
    let mut sup = Supervisor::open(root).await.unwrap();
    if backend {
        sup.set_pinned(WORLD, true).unwrap();
        sup.set_wake(WORLD, WakeMode::Operator).unwrap();
    }
    sup.start(WORLD, "parity-suite").await.unwrap();
    sup
}

#[tokio::test]
async fn a_local_host_and_a_hosted_node_agree_on_the_same_journal() {
    let fixture = TempDir::new().unwrap();
    let source = fixture.path().join(WORLD);
    let cast = write_fixture(&source).await;
    let journal = std::fs::read(source.join(JOURNAL_FILE)).unwrap();

    let local_root = TempDir::new().unwrap();
    let hosted_root = TempDir::new().unwrap();
    let backend_root = TempDir::new().unwrap();
    for root in [&local_root, &hosted_root, &backend_root] {
        copy_tree(&source, &root.path().join(WORLD));
    }

    let local_dir = local_root.path().join(WORLD);
    let (local, local_report) = open_local(&local_dir).await;
    let local_seen = observe(&local, &local_report, &local_dir, &cast).await;

    let mut verdicts = Vec::new();
    for (who, authority) in authorities(&cast) {
        for act in ACTS {
            verdicts.push((who, act, local_verdict(&local, act, &authority).await));
        }
    }
    // The oracle the design names: owner and operator wake, pause and
    // archive; restore is the operator's; nobody else may.
    for (who, act, verdict) in &verdicts {
        let allowed = *who == "operator" || (*who == "owner" && *act != LifecycleAct::Restore);
        assert_eq!(verdict.is_ok(), allowed, "{who} {act:?}: {verdict:?}");
    }

    for (root, backend) in [(&hosted_root, false), (&backend_root, true)] {
        let sup = hosted(root.path(), backend).await;
        let held = &sup.live[WORLD];
        let dir = root.path().join(WORLD);
        let seen = observe(&held.service, &held.report, &dir, &cast).await;
        assert_eq!(
            seen, local_seen,
            "hosted (backend settings: {backend}) diverged"
        );
        for (who, authority) in authorities(&cast) {
            for act in ACTS {
                let hosted = sup
                    .authorize(WORLD, act, &authority)
                    .await
                    .map_err(|error| error.to_string());
                let local = verdicts
                    .iter()
                    .find(|(w, a, _)| *w == who && *a == act)
                    .map(|(_, _, verdict)| verdict.clone())
                    .unwrap();
                assert_eq!(hosted, local, "{who} {act:?} (backend settings: {backend})");
            }
        }
        assert_eq!(sup.file().worlds[WORLD].pinned, backend);
        drop(sup);
        assert_eq!(
            std::fs::read(dir.join(JOURNAL_FILE)).unwrap(),
            journal,
            "replay never rewrites the journal"
        );
    }
    assert_eq!(
        std::fs::read(local_dir.join(JOURNAL_FILE)).unwrap(),
        journal
    );
    assert_eq!(
        HostCeilings::host(),
        HostCeilings::of(&WorldLimits::default(), &RuntimeLimits::default()),
        "a hosted world runs under the local defaults"
    );
    // The wake full-resync baseline (#805 will shrink it): what a member
    // receives on reconnecting to a woken world, as bytes on the wire.
    println!(
        "parity: full-resync snapshot bytes per wake for the fixture: {}",
        local_seen.snapshot.len()
    );
}

/// The same echo program the sandbox tests use: `plaza_update` returns
/// the participant id and never traps, `plaza_view` presents "hello".
#[cfg(feature = "sandbox")]
const ECHO: &str = r#"
    (module
      (memory (export "memory") 1 1)
      (data (i32.const 32) "hello")
      (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
      (func (export "plaza_free") (param i32 i32))
      (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
      (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
        local.get 0)
      (func (export "plaza_view") (param i32) (result i32)
        (i32.const 32))
      (func (export "plaza_out_len") (result i32) (i32.const 5)))
"#;

/// Everything world-visible about a program world on one side: the
/// replay of the installed program, the view it presents, the module
/// file it was restored from, and the world's own input ceiling.
#[cfg(feature = "sandbox")]
#[derive(Debug, PartialEq, Eq)]
struct ProgramObservation {
    sequence: u64,
    module_hash: Option<String>,
    replayed_inputs: usize,
    program_error: Option<String>,
    snapshot: Vec<u8>,
    view: Option<String>,
    modules: Vec<String>,
    over_ceiling: String,
    at_ceiling: bool,
}

#[cfg(feature = "sandbox")]
async fn observe_program(
    service: &WorldService,
    report: &OpenReport,
    dir: &Path,
    ann: &JoinCapability,
) -> ProgramObservation {
    let world = service.world();
    let snapshot = serde_json::to_vec(&world.snapshot(ann.clone()).await.unwrap()).unwrap();
    let view = world.view(ann.clone()).await.unwrap();
    let mut modules: Vec<String> = std::fs::read_dir(dir.join("modules"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    modules.sort();
    // The world's input ceiling, checked last because an accepted input
    // advances the world: one byte over is refused, the bound itself is
    // admitted, on every side alike.
    let bound = WorldLimits::default().input_bytes;
    let over_ceiling = world
        .input(ann.clone(), vec![b'x'; bound + 1])
        .await
        .expect_err("an input over the world's bound is refused")
        .to_string();
    let at_ceiling = world.input(ann.clone(), vec![b'x'; bound]).await.is_ok();
    ProgramObservation {
        sequence: report.sequence,
        module_hash: report.module_hash.clone(),
        replayed_inputs: report.replayed_inputs,
        program_error: report.program_error.clone(),
        snapshot,
        view,
        modules,
        over_ceiling,
        at_ceiling,
    }
}

/// Second fixture (#10698): a world with an installed Wasm program and
/// inputs it has applied. Local and hosted replays must restore the same
/// program from the same content-addressed module file, replay the same
/// inputs to the same view, and enforce the same input ceiling, and a
/// host-only setting must change none of it.
#[cfg(feature = "sandbox")]
#[tokio::test]
async fn a_program_world_replays_identically_on_a_local_host_and_a_hosted_node() {
    use crate::sandbox::{Sandbox, SandboxLimits};
    use crate::world_store::ModuleDir;

    const SEED: u64 = 7;
    let fixture = TempDir::new().unwrap();
    let source = fixture.path().join(WORLD);
    let wasm = wat::parse_str(ECHO).unwrap();
    let (service, _) = open_local(&source).await;
    let hash = ModuleDir::new(source.join("modules")).save(&wasm).unwrap();
    let program = Sandbox::compile(&wasm, SandboxLimits::default())
        .unwrap()
        .instantiate(SEED)
        .unwrap();
    let world = service.world();
    world
        .install_program(hash.clone(), SEED, Box::new(program))
        .await
        .unwrap();
    let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
    for input in [b"first".as_slice(), b"second", b"third"] {
        world.input(ann.clone(), input.to_vec()).await.unwrap();
    }
    world.shutdown().await.unwrap();
    let journal = std::fs::read(source.join(JOURNAL_FILE)).unwrap();

    let local_root = TempDir::new().unwrap();
    let hosted_root = TempDir::new().unwrap();
    let backend_root = TempDir::new().unwrap();
    for root in [&local_root, &hosted_root, &backend_root] {
        copy_tree(&source, &root.path().join(WORLD));
    }

    let local_dir = local_root.path().join(WORLD);
    let (local, local_report) = open_local(&local_dir).await;
    let local_seen = observe_program(&local, &local_report, &local_dir, &ann).await;
    // The oracle, independent of either side: the program restored from
    // the one module file it was installed from, with every input
    // replayed and nothing failing.
    assert_eq!(local_seen.module_hash.as_deref(), Some(hash.as_str()));
    assert_eq!(local_seen.modules, vec![format!("{hash}.wasm")]);
    assert_eq!(local_seen.replayed_inputs, 3);
    assert_eq!(local_seen.program_error, None);
    assert!(local_seen.at_ceiling);
    assert_eq!(
        local_seen.view.as_deref(),
        Some("hello"),
        "the replayed program presents its view"
    );

    for (root, backend) in [(&hosted_root, false), (&backend_root, true)] {
        let sup = hosted(root.path(), backend).await;
        let held = &sup.live[WORLD];
        let dir = root.path().join(WORLD);
        let seen = observe_program(&held.service, &held.report, &dir, &ann).await;
        assert_eq!(
            seen, local_seen,
            "hosted program world (backend settings: {backend}) diverged"
        );
        drop(sup);
        // Replay itself never rewrites the journal; only the two ceiling
        // probes, which every side ran alike, were appended after it.
        assert!(
            std::fs::read(dir.join(JOURNAL_FILE))
                .unwrap()
                .starts_with(&journal),
            "replay never rewrites the journal"
        );
    }
}
