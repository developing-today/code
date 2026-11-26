//! A serving node answers the operator CLI on `host.sock` (#10697).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;

use super::NodeControl;
use crate::host::{HostLifecycle, LifecycleState, WakeMode, open_in_root};
use crate::host_control::{ControlRequest, ControlResponse, bind, request};
use crate::world_hub::{HubLimits, OpenFuture, WorldHub, WorldOpener};

/// The host-mode opener `id serve --world-host` uses, in miniature: every
/// open is admitted by the host file and taken back when it fails, and an
/// idle close is recorded asleep.
struct HostOpener {
    root: PathBuf,
    host: Arc<HostLifecycle>,
}

impl WorldOpener for HostOpener {
    fn open<'a>(&'a self, name: &'a str, _create: bool) -> OpenFuture<'a> {
        Box::pin(async move {
            if !self.exists(name) {
                return Ok(None);
            }
            let admission = self.host.admit_wake(name)?;
            match open_in_root(&self.root, name).await {
                Ok((service, _)) => Ok(Some(service)),
                Err(error) => {
                    self.host.undo_wake(name, admission)?;
                    Err(error)
                }
            }
        })
    }

    fn exists(&self, name: &str) -> bool {
        self.root.join(name).join("journal.jsonl").is_file()
    }

    fn list(&self) -> Vec<String> {
        self.host.snapshot().worlds.into_keys().collect()
    }

    fn durable(&self) -> bool {
        true
    }

    fn closed(&self, name: &str) {
        self.host.record_closed(name).unwrap();
    }

    fn pinned(&self, name: &str) -> bool {
        self.host.pinned(name)
    }
}

fn data_root_with(names: &[&str]) -> TempDir {
    let root = TempDir::new().unwrap();
    for name in names {
        let dir = root.path().join(name);
        std::fs::create_dir_all(dir.join("modules")).unwrap();
        std::fs::write(dir.join("journal.jsonl"), "").unwrap();
    }
    root
}

fn node(root: &Path) -> (NodeControl, WorldHub, Arc<HostLifecycle>) {
    let host = Arc::new(HostLifecycle::open(root).unwrap());
    let opener = Arc::new(HostOpener {
        root: root.to_path_buf(),
        host: Arc::clone(&host),
    });
    let hub = WorldHub::new(opener, None, "alpha", HubLimits::default());
    let control = NodeControl::new(hub.clone(), Arc::clone(&host), None);
    (control, hub, host)
}

fn start(world: &str) -> ControlRequest {
    ControlRequest::Start {
        world: world.to_owned(),
        actor: "op".to_owned(),
    }
}

fn stop(world: &str) -> ControlRequest {
    ControlRequest::Stop {
        world: world.to_owned(),
        actor: "op".to_owned(),
    }
}

fn archive(world: &str) -> ControlRequest {
    ControlRequest::Archive {
        world: world.to_owned(),
        actor: "op".to_owned(),
    }
}

fn restore(world: &str) -> ControlRequest {
    ControlRequest::Restore {
        world: world.to_owned(),
        actor: "op".to_owned(),
    }
}

fn refusal(response: &ControlResponse) -> &str {
    assert!(!response.ok, "expected a refusal: {response:?}");
    response.error.as_deref().unwrap()
}

fn last_actor(host: &HostLifecycle, world: &str) -> String {
    host.snapshot().worlds[world]
        .history
        .last()
        .map(|change| change.actor.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn an_operator_start_opens_a_world_whatever_its_wake_mode() {
    let root = data_root_with(&["alpha", "beta"]);
    let (control, hub, host) = node(root.path());

    let set = control
        .handle(ControlRequest::SetWake {
            world: "alpha".to_owned(),
            wake: WakeMode::Operator,
        })
        .await;
    assert!(set.ok, "{set:?}");
    assert!(
        hub.lease(Some("alpha"), None).await.is_err(),
        "a client cannot wake an operator-only world"
    );
    assert_eq!(host.state("alpha"), Some(LifecycleState::Asleep));

    let started = control.handle(start("alpha")).await;
    assert!(started.ok, "{started:?}");
    assert_eq!(started.body.unwrap()["state"], "live");
    assert_eq!(host.state("alpha"), Some(LifecycleState::Live));
    assert_eq!(last_actor(&host, "alpha"), "op");
    assert!(hub.open_service("alpha").is_some(), "the hub holds it open");
    assert!(
        hub.lease(Some("alpha"), None).await.is_ok(),
        "a live world serves clients"
    );

    let again = control.handle(start("alpha")).await;
    assert!(refusal(&again).contains("already live"), "{again:?}");
    let busy = control.handle(archive("alpha")).await;
    assert!(!busy.ok, "a live world is not archived: {busy:?}");
}

#[tokio::test]
async fn an_operator_stop_drains_the_world_and_records_who_stopped_it() {
    let root = data_root_with(&["alpha"]);
    let (control, hub, host) = node(root.path());
    let held = hub.lease(Some("alpha"), None).await.unwrap();
    assert_eq!(last_actor(&host, "alpha"), crate::host::WAKE_ON_CONNECT);

    let stopped = control.handle(stop("alpha")).await;
    assert!(stopped.ok, "{stopped:?}");
    assert_eq!(stopped.body.unwrap()["state"], "asleep");
    assert_eq!(host.state("alpha"), Some(LifecycleState::Asleep));
    assert_eq!(last_actor(&host, "alpha"), "op");
    assert!(hub.open_service("alpha").is_none(), "the hub let it go");
    drop(held);

    let twice = control.handle(stop("alpha")).await;
    assert!(refusal(&twice).contains("already asleep"), "{twice:?}");

    // A client arriving after the stop wakes it again, on the record.
    hub.lease(Some("alpha"), None).await.unwrap();
    assert_eq!(host.state("alpha"), Some(LifecycleState::Live));
    assert_eq!(last_actor(&host, "alpha"), crate::host::WAKE_ON_CONNECT);
}

#[tokio::test]
async fn archive_and_restore_follow_the_lifecycle() {
    let root = data_root_with(&["alpha"]);
    let (control, hub, host) = node(root.path());

    assert!(control.handle(archive("alpha")).await.ok);
    assert_eq!(host.state("alpha"), Some(LifecycleState::Archived));
    assert!(
        hub.lease(Some("alpha"), None).await.is_err(),
        "an archived world never wakes"
    );
    let start_archived = control.handle(start("alpha")).await;
    assert!(
        refusal(&start_archived).contains("restore to asleep before starting"),
        "{start_archived:?}"
    );
    let stop_archived = control.handle(stop("alpha")).await;
    assert!(
        refusal(&stop_archived).contains("not live"),
        "a stop never restores: {stop_archived:?}"
    );
    assert_eq!(host.state("alpha"), Some(LifecycleState::Archived));

    assert!(control.handle(restore("alpha")).await.ok);
    assert_eq!(host.state("alpha"), Some(LifecycleState::Asleep));
    let restore_asleep = control.handle(restore("alpha")).await;
    assert!(!restore_asleep.ok, "{restore_asleep:?}");

    let unknown = control.handle(start("nope")).await;
    assert!(refusal(&unknown).contains("no world"), "{unknown:?}");
}

#[tokio::test]
async fn an_export_needs_a_live_world_and_a_key() {
    let root = data_root_with(&["alpha"]);
    let (control, _hub, _host) = node(root.path());
    let path = root.path().join("alpha.export");
    let export = || ControlRequest::Export {
        world: "alpha".to_owned(),
        path: path.clone(),
    };
    let asleep = control.handle(export()).await;
    assert!(refusal(&asleep).contains("not live"), "{asleep:?}");
    assert!(control.handle(start("alpha")).await.ok);
    let keyless = control.handle(export()).await;
    assert!(refusal(&keyless).contains("signing key"), "{keyless:?}");
    assert!(!path.exists());
}

#[tokio::test]
async fn the_cli_reaches_a_serving_node_over_its_socket() {
    let root = data_root_with(&["alpha", "beta"]);
    let (control, hub, host) = node(root.path());
    let listener = bind(root.path()).unwrap();
    assert!(
        bind(root.path()).is_err(),
        "a second host on the same root is refused while the node answers"
    );
    let (done, finished) = tokio::sync::oneshot::channel::<()>();
    let served = tokio::spawn(async move {
        control
            .serve(&listener, async {
                let _ = finished.await;
            })
            .await
    });

    let listed = request(root.path(), &ControlRequest::List).await.unwrap();
    assert!(listed.ok, "{listed:?}");
    assert_eq!(listed.body.unwrap()["asleep"]["worlds"], 2);

    let started = request(root.path(), &start("beta")).await.unwrap();
    assert!(started.ok, "{started:?}");
    assert!(hub.open_service("beta").is_some());

    let inspected = request(
        root.path(),
        &ControlRequest::Inspect {
            world: "beta".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(inspected.body.unwrap()["state"], "live");

    let pinned = request(
        root.path(),
        &ControlRequest::SetPinned {
            world: "beta".to_owned(),
            pinned: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(pinned.body.unwrap()["pinned"], true);
    assert!(host.pinned("beta"));

    let stopped = request(root.path(), &stop("beta")).await.unwrap();
    assert!(stopped.ok, "{stopped:?}");
    assert_eq!(host.state("beta"), Some(LifecycleState::Asleep));

    done.send(()).unwrap();
    served.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_serving_node_takes_scheduled_snapshots_of_its_live_worlds() {
    use crate::host_snapshots::{SnapshotPolicy, list_sets};

    let root = data_root_with(&["alpha", "beta"]);
    let (keyless, hub, host) = node(root.path());
    let control = NodeControl::new(
        hub,
        Arc::clone(&host),
        Some(ed25519_dalek::SigningKey::from_bytes(&[7; 32])),
    );
    let hourly = SnapshotPolicy {
        every_secs: 3_600,
        keep: 2,
        max_age_secs: None,
    };
    let set = control
        .handle(ControlRequest::SetSnapshots {
            world: None,
            policy: Some(hourly),
        })
        .await;
    assert!(set.ok, "{set:?}");
    assert_eq!(host.snapshot().snapshot_policy("alpha"), hourly);
    let bad = control
        .handle(ControlRequest::SetSnapshots {
            world: None,
            policy: Some(SnapshotPolicy { keep: 0, ..hourly }),
        })
        .await;
    assert!(!bad.ok, "a policy keeping nothing is refused: {bad:?}");

    assert!(control.handle(start("alpha")).await.ok);
    // A node without a key takes nothing; asleep worlds are never taken.
    assert!(keyless.run_due_snapshots(1_000).await.is_empty());
    let run = control.run_due_snapshots(1_000).await;
    assert!(run.failed.is_empty(), "{:?}", run.failed);
    assert_eq!(
        run.taken
            .iter()
            .map(|set| set.world.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha"]
    );
    // Not due again inside the cadence.
    assert!(control.run_due_snapshots(2_000).await.is_empty());

    let snapshot_root = host.snapshot().snapshot_root(root.path());
    assert_eq!(list_sets(&snapshot_root, "alpha").unwrap().len(), 1);
    let shown = control
        .handle(ControlRequest::Snapshots {
            world: Some("alpha".to_owned()),
        })
        .await;
    assert!(shown.ok, "{shown:?}");
    assert_eq!(shown.body.unwrap()["sets"].as_array().unwrap().len(), 1);
}
