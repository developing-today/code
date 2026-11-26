//! The serving node's answer on `host.sock` (#10697, packet P5 of the
//! hosted-worlds v1 design `doc/2026-11-16_design_id6283_hosted_worlds_v1.md`).
//!
//! `id serve --world-host` serves a data root through a [`WorldHub`] whose
//! opener records every wake and idle close in the root's host file
//! ([`HostLifecycle`]). A deployed node is that one process, so it answers
//! the same control socket `id host run` binds: `id host start`, `stop`,
//! `archive`, `restore`, `list`, `inspect`, `pin`, `wake`, `export` and
//! `import` act on the worlds the node is serving, with the same request
//! and response lines ([`ControlRequest`], [`ControlResponse`]).
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

use anyhow::{Context, Result, bail};
use ed25519_dalek::SigningKey;
use tokio::net::UnixListener;

use crate::host::{HostLifecycle, LifecycleState, census_of, inspect_world, settings_body};
use crate::host_control::{
    ControlRequest, ControlResponse, export_body, parse_source, read_request, state_body,
    write_response,
};
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

    /// Answer requests on `listener` one at a time until `shutdown`
    /// resolves.
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
        loop {
            tokio::select! {
                () = &mut shutdown => return Ok(()),
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
