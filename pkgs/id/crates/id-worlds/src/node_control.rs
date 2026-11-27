//! The serving node's answer on `host.sock` (#10697, packet P5 of the
//! hosted-worlds v1 design `doc/2026-11-16_design_id6283_hosted_worlds_v1.md`).
//!
//! `id serve --world-host` serves a data root through a [`WorldHub`] whose
//! opener records every wake and idle close in the root's host file
//! ([`HostLifecycle`]). A deployed node is that one process, so it answers
//! the same control socket `id host run` binds: `id host start`, `stop`,
//! `archive`, `restore`, `list`, `inspect`, `pin`, `wake`, `export` and
//! `import` act on the worlds the node is serving, with the same request
//! and response lines ([`ControlRequest`], [`ControlResponse`]). `id host
//! snapshots` sets the node's snapshot policy, and the node takes and
//! prunes scheduled sets of its live worlds as `id host run` does
//! (#13739).
//!
//! A start records the operator's act first and then opens the world in
//! the hub, so it works whatever the world's wake mode is; a failed open
//! takes the record back. A stop records the world asleep first and then
//! drains it out of the hub, so a session arriving between the two wakes
//! it again on the record rather than finding it live on paper alone.
//! Requests are answered one at a time, in the order they arrive.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use tokio::net::UnixListener;

use crate::host::{
    HostLifecycle, LifecycleState, census_of, inspect_world, settings_body, snapshot_live_world,
    snapshot_settings_body,
};
use crate::host_control::{
    ControlRequest, ControlResponse, export_body, log_snapshot_run, parse_source, read_request,
    state_body, write_response,
};
use crate::host_snapshots::{SNAPSHOT_TICK_SECS, SnapshotRun};
use crate::world_export::export_world_to_file;
use crate::world_hub::WorldHub;

/// The operator's control over a serving node: the hub that holds its open
/// worlds, the host file that records them, and the key that signs exports.
#[derive(Clone)]
pub struct NodeControl {
    hub: WorldHub,
    host: Arc<HostLifecycle>,
    signing_key: Option<SigningKey>,
}

impl std::fmt::Debug for NodeControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeControl")
            .field("root", &self.host.root())
            .field("exports", &self.signing_key.is_some())
            .finish_non_exhaustive()
    }
}

impl NodeControl {
    /// Control over the worlds `hub` serves from the data root `host`
    /// records. Without `signing_key`, exports are refused.
    #[must_use]
    pub const fn new(
        hub: WorldHub,
        host: Arc<HostLifecycle>,
        signing_key: Option<SigningKey>,
    ) -> Self {
        Self {
            hub,
            host,
            signing_key,
        }
    }

    /// The data root under control.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.host.root()
    }

    /// Perform one request against the node.
    pub async fn handle(&self, request: ControlRequest) -> ControlResponse {
        match self.act(request).await {
            Ok(body) => ControlResponse::done(body),
            Err(error) => ControlResponse::refused(format!("{error:#}")),
        }
    }

    async fn act(&self, request: ControlRequest) -> Result<serde_json::Value> {
        match request {
            ControlRequest::List => Ok(serde_json::to_value(census_of(
                self.host.root(),
                &self.host.snapshot(),
            ))?),
            ControlRequest::Inspect { world } => Ok(serde_json::to_value(inspect_world(
                self.host.root(),
                &self.host.snapshot(),
                &world,
            )?)?),
            ControlRequest::Start { world, actor } => self.start(&world, &actor).await,
            ControlRequest::Stop { world, actor } => self.stop(&world, &actor).await,
            ControlRequest::Archive { world, actor } => {
                self.host.operator_transition(
                    &world,
                    LifecycleState::Asleep,
                    LifecycleState::Archived,
                    &actor,
                )?;
                Ok(state_body(&world, LifecycleState::Archived))
            }
            ControlRequest::Restore { world, actor } => {
                self.host.operator_transition(
                    &world,
                    LifecycleState::Archived,
                    LifecycleState::Asleep,
                    &actor,
                )?;
                Ok(state_body(&world, LifecycleState::Asleep))
            }
            ControlRequest::SetPinned { world, pinned } => {
                self.host.set_pinned(&world, pinned)?;
                settings_body(&self.host.snapshot(), &world)
            }
            ControlRequest::SetWake { world, wake } => {
                self.host.set_wake(&world, wake)?;
                settings_body(&self.host.snapshot(), &world)
            }
            ControlRequest::SetSnapshots { world, policy } => {
                self.host.set_snapshot_policy(world.as_deref(), policy)?;
                snapshot_settings_body(self.host.root(), &self.host.snapshot(), world.as_deref())
            }
            ControlRequest::Snapshots { world } => {
                snapshot_settings_body(self.host.root(), &self.host.snapshot(), world.as_deref())
            }
            ControlRequest::Export { world, path } => {
                let service = self.hub.open_service(&world).with_context(|| {
                    format!("world {world:?} is not live: start it before exporting")
                })?;
                let key = self
                    .signing_key
                    .as_ref()
                    .context("this host has no signing key for exports")?;
                let manifest = export_world_to_file(&service, key, &path).await?;
                export_body(&manifest, &path)
            }
            ControlRequest::Import {
                path,
                source,
                actor,
            } => {
                let source = parse_source(&source)?;
                let manifest = self.host.import(&path, source, &actor).await?;
                export_body(&manifest, &path)
            }
        }
    }

    async fn start(&self, world: &str, actor: &str) -> Result<serde_json::Value> {
        self.host.operator_transition(
            world,
            LifecycleState::Asleep,
            LifecycleState::Live,
            actor,
        )?;
        // The opener admits a world already recorded live, so the hub opens
        // it whatever its wake mode says.
        match self.hub.lease(Some(world), None).await {
            Ok(lease) => {
                drop(lease);
                Ok(state_body(world, LifecycleState::Live))
            }
            Err(error) => {
                if let Err(undo) = self.host.undo_start(world, actor) {
                    tracing::warn!(world, "host control: could not undo start: {undo:#}");
                }
                bail!("starting world {world:?}: {}", error.message())
            }
        }
    }

    async fn stop(&self, world: &str, actor: &str) -> Result<serde_json::Value> {
        self.host.operator_transition(
            world,
            LifecycleState::Live,
            LifecycleState::Asleep,
            actor,
        )?;
        self.hub
            .close(world)
            .await
            .with_context(|| format!("shutting down world {world:?}"))?;
        Ok(state_body(world, LifecycleState::Asleep))
    }

    /// Take a set of every world the node serves live whose snapshot
    /// policy says it is due at `now_secs`, then prune its sets (#13739).
    ///
    /// A node with no signing key takes nothing. One world failing is
    /// recorded in the run and never stops the others.
    pub async fn run_due_snapshots(&self, now_secs: u64) -> SnapshotRun {
        let mut run = SnapshotRun::default();
        let Some(key) = self.signing_key.as_ref() else {
            return run;
        };
        let file = self.host.snapshot();
        let snapshot_root = file.snapshot_root(self.host.root());
        for (name, record) in &file.worlds {
            if record.state != LifecycleState::Live {
                continue;
            }
            let policy = file.snapshot_policy(name);
            if !policy.is_on() {
                continue;
            }
            // A world recorded live but not open in the hub has nothing to
            // export until a session wakes it.
            let Some(service) = self.hub.open_service(name) else {
                continue;
            };
            if let Err(error) = snapshot_live_world(
                &service,
                key,
                name,
                &policy,
                &snapshot_root,
                now_secs,
                &mut run,
            )
            .await
            {
                run.failed.push((name.clone(), format!("{error:#}")));
            }
        }
        run
    }

    /// Answer requests on `listener` one at a time until `shutdown`
    /// resolves.
    /// Between requests it takes the scheduled snapshots that are due.
    ///
    /// # Errors
    ///
    /// Never for a bad request or a client that hangs up: those are
    /// logged and the next connection is answered.
    pub async fn serve(
        &self,
        listener: &UnixListener,
        shutdown: impl Future<Output = ()>,
    ) -> Result<()> {
        tokio::pin!(shutdown);
        let mut snapshots = tokio::time::interval(Duration::from_secs(SNAPSHOT_TICK_SECS));
        snapshots.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = &mut shutdown => return Ok(()),
                _ = snapshots.tick() => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |since| since.as_secs());
                    log_snapshot_run(&self.run_due_snapshots(now).await);
                }
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        if let Err(error) = self.answer(stream).await {
                            tracing::warn!("host control: {error:#}");
                        }
                    }
                    Err(error) => tracing::warn!("host control: accept failed: {error}"),
                },
            }
        }
    }

    async fn answer(&self, stream: tokio::net::UnixStream) -> Result<()> {
        let (request, write) = read_request(stream).await?;
        let response = match request {
            Ok(request) => self.handle(request).await,
            Err(refusal) => refusal,
        };
        write_response(write, &response).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#[path = "node_control_tests.rs"]
mod tests;
