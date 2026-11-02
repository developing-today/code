//! Transport-neutral world session driver.
//!
//! A *session* is one participant's connection to a [`WorldHandle`]. The
//! wire protocol is one JSON object per frame; this module owns that protocol
//! and nothing else. A transport (WebSocket, Iroh stream, an in-memory test
//! channel) only implements [`SessionIo`] to move whole text frames, so the
//! behavior cannot drift between transports.
//!
//! Client frames (`type` tag): `join {capability, after?}`,
//! `invite {admin_token, display_name}`, `attenuate {name, scopes}`, `chat {text}`, `input {data_hex}`.
//! The first frame must be `join` or `invite`. Server frames: `snapshot`,
//! `event`, `invite`, `error`.
//!
//! Authorization is the capability alone. A transport may know who the peer
//! is (an Iroh node ID); that never grants world authority.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use iroh_blobs::{
    BlobFormat, Hash,
    api::{Store, blobs::AddBytesOptions},
};
use rand::RngExt as _;
use serde::Serialize;
use subtle::ConstantTimeEq as _;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore, broadcast};

use crate::directory_auth::Signed;
use crate::time::unix_ms;
use crate::world::{
    Authority, CapabilityBounds, FileScope, Isolation, JoinCapability, Participant, WorldHandle,
    WorldMetrics, WorldPolicy, WorldScopes,
};
use crate::world_compile::Compiler;
#[cfg(feature = "sandbox")]
use crate::world_compile::{COMPILE_TIMEOUT, CompileSpec};
use crate::world_hub::{WorldHub, WorldLease};

/// Bound the extra presentation latency while coalescing view-only redraws.
const VIEW_COALESCE_WINDOW: Duration = Duration::from_millis(5);

/// Re-exported so `crate::world_session::MAX_FRAME_BYTES` keeps working: the
/// limit now lives in [`crate::frames`] so the crate builds without the
/// `world` feature.
pub use crate::frames::MAX_FRAME_BYTES;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
/// Largest Wasm world module that may be installed (16 MiB).
mod types;

pub(crate) use types::{
    CAPS_CEILING_NOTICE, ClientFrame, MAX_ADMIN_TOKEN_BYTES, MAX_CAPABILITY_BYTES,
    MAX_CONCURRENT_MODULE_UPLOADS, ServerFrame, invite_bounds,
};
pub use types::{
    INVITE_DEFAULT_SECS, INVITE_MAX_SECS, Inbound, InstallError, InviteError, InviteResponse,
    MAX_WORLD_MODULE_BYTES, MAX_WORLD_MODULE_CHUNK_BYTES, SessionConfig, SessionIo, WorldService,
    check_invite_bounds,
};

impl WorldService {
    /// Accept native (ELF) module installs. Wasm stays the default tier.
    #[must_use]
    pub const fn with_native_enabled(mut self) -> Self {
        self.native = true;
        self
    }

    /// Require new Wasm programs to be compiled from source by this server.
    /// This disables raw artifact installation, but not the compiled output path.
    #[must_use]
    pub const fn with_source_only(mut self, enabled: bool) -> Self {
        self.source_only = enabled;
        self
    }

    /// Whether raw Wasm/native uploads are disabled for this world.
    #[must_use]
    pub(crate) const fn source_only(&self) -> bool {
        self.source_only
    }

    /// Read or change this world's directory as the caller.
    ///
    /// # Errors
    ///
    /// Fails for an unknown credential or a refused change.
    pub async fn directory(
        &self,
        caller: crate::directory_auth::Caller,
        action: crate::directory_view::DirectoryAction,
    ) -> anyhow::Result<crate::directory_view::DirectoryOutcome> {
        if action.sends_mail() && self.mail.is_none() {
            return Err(crate::directory::Refusal::Forbidden(
                "this server has no mail configured".to_owned(),
            )
            .into());
        }
        // The actor refuses an outbox-less server only when the change would queue for another server.
        let mut outcome = self
            .world
            .directory(caller.clone(), action, self.outbox.is_some())
            .await?;
        if let (Some(sink), Some(mail)) = (&self.mail, outcome.mail.take()) {
            let sink = Arc::clone(sink);
            tokio::task::spawn_blocking(move || sink.send(&mail))
                .await
                .context("mail task stopped")??;
            outcome.mailed = true;
        }
        if let Some(outbox) = &self.outbox {
            let outbound = std::mem::take(&mut outcome.outbound);
            match outcome.pending.take() {
                Some(entry) => {
                    self.queue_then_apply(outbox, outbound, entry).await?;
                    outcome = self
                        .world
                        .directory(caller, crate::directory_view::DirectoryAction::View, true)
                        .await?;
                }
                None => {
                    enqueue_all(&mut *outbox.lock().await, outbound)?;
                }
            }
            let queue = outbox.lock().await;
            outcome.view.deliveries =
                crate::directory_view::deliveries(&queue, &outcome.view.viewer);
        }
        Ok(outcome)
    }

    /// Queue the outbound payloads, then apply the change. The outbox lock is
    /// held across the apply so a flush cannot send anything before the change
    /// is in. If the apply is refused, the payloads are withdrawn.
    async fn queue_then_apply(
        &self,
        outbox: &Mutex<crate::envelope_outbox::Outbox>,
        outbound: Vec<crate::directory_view::Outbound>,
        entry: crate::directory::DirectoryEntry,
    ) -> anyhow::Result<()> {
        let mut queue = outbox.lock().await;
        let keys = enqueue_all(&mut queue, outbound)?;
        if let Err(error) = self.world.apply_directory_entry(entry).await {
            withdraw_all(&mut queue, &keys);
            return Err(error);
        }
        Ok(())
    }

    /// Attempt every due outbox envelope once. Returns how many were attempted.
    ///
    /// # Errors
    ///
    /// Fails when the outbox file cannot be written.
    pub async fn flush_outbox(&self, endpoint: &iroh::Endpoint, now: u64) -> anyhow::Result<usize> {
        match &self.outbox {
            Some(outbox) => crate::envelope_outbox::flush(outbox, endpoint, now).await,
            None => Ok(0),
        }
    }

    /// Wrap a world. `admin_token: None` disables invites.
    #[must_use]
    pub fn new(world: WorldHandle, admin_token: Option<String>) -> Self {
        Self {
            metrics: world.shared_metrics(),
            world,
            admin_token,
            blobs: None,
            module_pin: Arc::new(Mutex::new(None)),
            module_hash: Arc::new(RwLock::new(None)),
            upload_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_MODULE_UPLOADS)),
            module_dir: None,
            records: None,
            compiler: None,
            native: false,
            source_only: false,
            runtime: crate::world_limits::RuntimeLimits::default(),
            mail: None,
            outbox: None,
            profile_operation: Arc::new(Mutex::new(())),
        }
    }

    /// The world's host metrics registry (F11.7), for metering the
    /// frames this service sends.
    pub(crate) fn frame_meter(&self) -> &WorldMetrics {
        &self.metrics
    }

    /// Bound the programs this world runs by the admin's limits.
    #[must_use]
    pub const fn with_runtime_limits(
        mut self,
        runtime: crate::world_limits::RuntimeLimits,
    ) -> Self {
        self.runtime = runtime;
        self
    }

    /// Replace the world's capability ceiling, admin only. Unlike a profile,
    /// the ceiling applies at once: the next use or subscription sees it.
    ///
    /// # Errors
    ///
    /// Fails with `admin denied` for a wrong token, and with the world's
    /// message for an invalid or unstorable ceiling.
    pub async fn set_caps_ceiling(
        &self,
        supplied_admin: Option<&str>,
        ceiling: Vec<String>,
    ) -> anyhow::Result<crate::world_caps::CapsReport> {
        anyhow::ensure!(self.authorize_admin(supplied_admin).is_ok(), "admin denied");
        self.world.set_caps_ceiling(ceiling).await
    }

    /// Pend one schedule for this world, admin only, journaled before the
    /// reply. Replaces any pending schedule with the same id. Returns the
    /// full pending map (id to fire time, unix ms).
    ///
    /// # Errors
    ///
    /// Fails with `admin denied` for a wrong token, and with the world's
    /// message for an invalid or unstorable schedule.
    pub async fn set_schedule(
        &self,
        supplied_admin: Option<&str>,
        id: String,
        at_ms: u64,
    ) -> anyhow::Result<std::collections::BTreeMap<String, u64>> {
        anyhow::ensure!(self.authorize_admin(supplied_admin).is_ok(), "admin denied");
        self.world.admin_schedule_at(id, at_ms).await
    }

    /// Cancel a pending schedule for this world, admin only, journaled
    /// before the reply. A no-op when the id is not pending. Returns the
    /// full pending map.
    ///
    /// # Errors
    ///
    /// Fails with `admin denied` for a wrong token, and with the world's
    /// message for a malformed id or storage failure.
    pub async fn cancel_schedule(
        &self,
        supplied_admin: Option<&str>,
        id: String,
    ) -> anyhow::Result<std::collections::BTreeMap<String, u64>> {
        anyhow::ensure!(self.authorize_admin(supplied_admin).is_ok(), "admin denied");
        self.world.admin_cancel_schedule(id).await
    }

    /// Persist a per-world resource profile. The running program keeps its
    /// existing limits; the request applies on the next install or restart.
    pub async fn set_execution_profile(
        &self,
        supplied_admin: Option<&str>,
        profile: crate::world_limits::WorldExecutionProfile,
    ) -> anyhow::Result<crate::world::ExecutionProfileUpdate> {
        self.authorize_admin(supplied_admin)
            .map_err(|_| anyhow::anyhow!("admin denied"))?;
        // Resolve now for validation and to ensure this request is valid under
        // this process's hard ceilings. The stored request remains unclamped,
        // so a later server with lower ceilings still wins at instantiation.
        profile.resolve(self.runtime)?;
        let _operation = self.profile_operation.lock().await;
        self.world.set_execution_profile(profile).await
    }

    /// Let admins compile Roc sources into this world on the fly.
    #[must_use]
    pub fn with_compiler(mut self, compiler: Compiler) -> Self {
        self.compiler = Some(Arc::new(compiler));
        self
    }

    /// Compile `spec` on the server and install the result exactly like an
    /// admin upload (validated, pinned, journaled). Returns the module hash,
    /// the install sequence and the compiler's diagnostics.
    ///
    /// # Errors
    ///
    /// A human-readable refusal: unauthorized, disabled, or the compiler's
    /// own failure text (so an admin sees why their program did not build).
    #[cfg(feature = "sandbox")]
    pub async fn compile_and_install(
        &self,
        supplied_admin: Option<&str>,
        spec: &CompileSpec,
    ) -> Result<(String, u64, String), String> {
        if self.authorize_admin(supplied_admin).is_err() {
            return Err("compile denied".to_owned());
        }
        let Some(compiler) = self.compiler.as_ref() else {
            return Err("compilation is not enabled on this server".to_owned());
        };
        if spec.native && !self.native {
            return Err("native modules are not enabled on this server".to_owned());
        }
        // The effective list, not the raw grants: a platform module outside a
        // server or world ceiling is refused at compile time too.
        let granted = self
            .world
            .caps_report()
            .await
            .map_err(|error| format!("compile failed: {error:#}"))?
            .effective;
        let (wasm, diagnostics, provenance) =
            crate::world_compile::compile(spec, compiler, &granted, COMPILE_TIMEOUT)
                .await
                .map_err(|error| {
                    tracing::warn!("world compile failed: {error:#}");
                    format!("compile failed: {error:#}")
                })?;
        let hash = module_hash(&wasm);
        let permit = self
            .try_reserve_module_upload()
            .map_err(|error| format!("install failed: {error:?}"))?;
        let (_, sequence) = self
            .install_wasm_reserved(
                supplied_admin,
                wasm,
                spec.seed,
                &hash,
                permit,
                true,
                Some(provenance),
            )
            .await
            .map_err(|error| format!("install failed: {error:?}"))?;
        Ok((hash, sequence, diagnostics))
    }

    /// Attach the world's records document and start mirroring program
    /// records into it.
    #[must_use]
    pub fn with_records_store(mut self, records: crate::world_records::RecordsStore) -> Self {
        let records = Arc::new(records);
        records.spawn_publisher(&self.world);
        self.records = Some(records);
        self
    }

    /// The world's records document, if it has one.
    #[must_use]
    pub const fn records_store(&self) -> Option<&Arc<crate::world_records::RecordsStore>> {
        self.records.as_ref()
    }

    /// Persist installed modules here so a durable world can re-instantiate
    /// its program after a restart.
    #[must_use]
    pub fn with_module_dir(mut self, modules: crate::world_store::ModuleDir) -> Self {
        self.module_dir = Some(modules);
        self
    }

    /// Mark `wasm` as the active module after a restore: pin it in the blob
    /// store (when attached) so peers can download it again.
    ///
    /// # Errors
    ///
    /// Fails if the blob store refuses the bytes.
    pub async fn adopt_module(&self, wasm: Vec<u8>) -> anyhow::Result<()> {
        let hash = module_hash(&wasm);
        if let Some(blobs) = self.blobs.as_ref() {
            let pin = blobs
                .add_bytes_with_opts(AddBytesOptions {
                    data: wasm.into(),
                    format: BlobFormat::Raw,
                })
                .temp_tag()
                .await?;
            *self.module_pin.lock().await = Some(pin);
        }
        *self.module_hash.write().await = Some(hash);
        Ok(())
    }

    /// Attach this node's blob store so uploaded modules survive as pinned
    /// Iroh content for the life of the running world.
    #[must_use]
    pub fn with_blob_store(mut self, blobs: Store) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// Send confirmation and sign-in codes through this sink. Without one,
    /// email actions are refused.
    #[must_use]
    pub fn with_mail(mut self, sink: Arc<dyn crate::directory_mail::MailSink>) -> Self {
        self.mail = Some(sink);
        self
    }

    /// Queue envelopes for other servers in this outbox. Without one, remote
    /// friend actions are refused.
    #[must_use]
    pub fn with_outbox(mut self, outbox: crate::envelope_outbox::Outbox) -> Self {
        self.outbox = Some(Arc::new(Mutex::new(outbox)));
        self
    }

    /// Reserve one upload slot before accepting a potentially large body.
    fn try_reserve_module_upload(&self) -> Result<OwnedSemaphorePermit, InstallError> {
        Arc::clone(&self.upload_slots)
            .try_acquire_owned()
            .map_err(|_| InstallError::Unavailable)
    }

    /// The underlying world actor.
    #[must_use]
    pub const fn world(&self) -> &WorldHandle {
        &self.world
    }

    /// Mint a guest capability if `supplied` matches the admin secret.
    ///
    /// # Errors
    ///
    /// See [`InviteError`].
    pub async fn invite(
        &self,
        supplied: Option<&str>,
        display_name: String,
        expires_in_secs: Option<u64>,
        uses: Option<u32>,
    ) -> Result<InviteResponse, InviteError> {
        self.invite_scoped(
            supplied,
            display_name,
            expires_in_secs,
            uses,
            WorldScopes::GUEST,
        )
        .await
    }

    /// Like [`Self::invite`], with the invited participant's scopes chosen by
    /// the caller (see `WorldScopes::from_role`).
    pub async fn invite_scoped(
        &self,
        supplied: Option<&str>,
        display_name: String,
        expires_in_secs: Option<u64>,
        uses: Option<u32>,
        scopes: WorldScopes,
    ) -> Result<InviteResponse, InviteError> {
        self.authorize_admin(supplied)?;
        let bounds =
            invite_bounds(expires_in_secs, uses, unix_ms()).map_err(InviteError::InvalidBounds)?;
        match self.world.issue_bounded(display_name, scopes, bounds).await {
            Ok((participant, capability)) => Ok(InviteResponse {
                capability: capability.expose().to_owned(),
                participant_id: participant.id,
                display_name: participant.display_name,
            }),
            Err(_) => Err(InviteError::Unavailable),
        }
    }

    /// The configured admin secret, for a hub that shares it process-wide.
    pub(crate) fn admin_token(&self) -> Option<&str> {
        self.admin_token.as_deref()
    }

    /// Validate the separately configured world-administrator token.
    pub fn authorize_admin(&self, supplied: Option<&str>) -> Result<(), InviteError> {
        let Some(expected) = self.admin_token.as_deref().filter(|t| !t.is_empty()) else {
            return Err(InviteError::Disabled);
        };
        if !supplied.is_some_and(|s| {
            s.len() <= MAX_ADMIN_TOKEN_BYTES && secret_eq(expected.as_bytes(), s.as_bytes())
        }) {
            return Err(InviteError::Unauthorized);
        }
        Ok(())
    }

    /// Compile, instantiate and atomically install a Wasm program. The module
    /// is first pinned in Iroh's blob store; only then does the actor activate
    /// it. A failed compile or write leaves the running program unchanged.
    #[cfg(feature = "sandbox")]
    pub async fn install_wasm(
        &self,
        supplied_admin: Option<&str>,
        wasm: Vec<u8>,
        seed: u64,
        claimed_hash: &str,
    ) -> Result<(String, u64), InstallError> {
        let permit = self.try_reserve_module_upload()?;
        self.install_wasm_reserved(
            supplied_admin,
            wasm,
            seed,
            claimed_hash,
            permit,
            false,
            None,
        )
        .await
    }

    #[cfg(feature = "sandbox")]
    async fn install_wasm_reserved(
        &self,
        supplied_admin: Option<&str>,
        wasm: Vec<u8>,
        seed: u64,
        claimed_hash: &str,
        _permit: OwnedSemaphorePermit,
        compiled_from_source: bool,
        provenance: Option<crate::world::ModuleProvenance>,
    ) -> Result<(String, u64), InstallError> {
        let _profile_operation = self.profile_operation.lock().await;
        self.authorize_admin(supplied_admin)
            .map_err(|error| match error {
                InviteError::Disabled => InstallError::Disabled,
                InviteError::Unauthorized => InstallError::Unauthorized,
                InviteError::Unavailable => InstallError::Unavailable,
                InviteError::InvalidBounds(_) => InstallError::InvalidUpload,
            })?;
        if self.source_only && !compiled_from_source {
            return Err(InstallError::RawModuleDisabled);
        }
        if compiled_from_source && provenance.is_none() {
            return Err(InstallError::InvalidModule);
        }
        if wasm.is_empty() || wasm.len() > MAX_WORLD_MODULE_BYTES {
            return Err(InstallError::InvalidUpload);
        }
        let hash = Hash::new(&wasm);
        let module_hash = hash.to_string();
        if claimed_hash != module_hash {
            return Err(InstallError::InvalidUpload);
        }
        let is_native = crate::world_native::is_native_module(&wasm);
        if is_native && !self.native {
            return Err(InstallError::InvalidModule);
        }
        let guest: Box<dyn crate::world::WorldProgram> = if is_native {
            // A durable world must have the worker on disk before the journal
            // references it; native modules spawn from their stored file.
            let modules = self.module_dir.clone().ok_or(InstallError::Disabled)?;
            let bytes = wasm.clone();
            let saved = tokio::task::spawn_blocking(move || modules.save(&bytes))
                .await
                .map_err(|_| InstallError::Unavailable)?
                .map_err(|_| InstallError::Unavailable)?;
            if saved != module_hash {
                return Err(InstallError::Unavailable);
            }
            let path = self
                .module_dir
                .as_ref()
                .ok_or(InstallError::Disabled)?
                .path_of(&module_hash)
                .map_err(|_| InstallError::Unavailable)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                    .await
                    .map_err(|_| InstallError::Unavailable)?;
            }
            let runtime = self
                .world
                .execution_profile()
                .await
                .map_err(|_| InstallError::Unavailable)?
                .resolve(self.runtime)
                .map_err(|_| InstallError::InvalidModule)?;
            let native_limits = runtime.native;
            let program = tokio::task::spawn_blocking(move || {
                crate::world_native::NativeProgram::new(&path, seed, native_limits, None)
            })
            .await
            .map_err(|e| {
                tracing::error!("native install join failed: {e:#}");
                InstallError::InvalidModule
            })?
            .map_err(|e| {
                tracing::error!("native install failed: {e:#}");
                InstallError::InvalidModule
            })?;
            Box::new(program)
        } else {
            let runtime = self
                .world
                .execution_profile()
                .await
                .map_err(|_| InstallError::Unavailable)?
                .resolve(self.runtime)
                .map_err(|_| InstallError::InvalidModule)?;
            let limits = runtime.sandbox;
            let compile_bytes = wasm.clone();
            let program = tokio::task::spawn_blocking(move || {
                crate::sandbox::Sandbox::compile(&compile_bytes, limits)?.instantiate(seed)
            })
            .await
            .map_err(|_| InstallError::InvalidModule)?
            .map_err(|_| InstallError::InvalidModule)?;
            Box::new(program)
        };

        // A durable world must be able to reload this module before the
        // journal may reference it (native was saved above).
        if !is_native && let Some(modules) = self.module_dir.clone() {
            let bytes = wasm.clone();
            let saved = tokio::task::spawn_blocking(move || modules.save(&bytes))
                .await
                .map_err(|_| InstallError::Unavailable)?
                .map_err(|_| InstallError::Unavailable)?;
            if saved != module_hash {
                return Err(InstallError::Unavailable);
            }
        }

        // Native workers are never offered for peer download: only the Wasm
        // tier gets pinned in the blob store.
        let blobs = self.blobs.as_ref().ok_or(InstallError::Disabled)?.clone();
        let pin = (!is_native).then(|| {
            blobs
                .add_bytes_with_opts(AddBytesOptions {
                    data: wasm.into(),
                    format: BlobFormat::Raw,
                })
                .temp_tag()
        });
        let pin = match pin {
            Some(pin) => {
                let pin = pin.await.map_err(|_| InstallError::Unavailable)?;
                if pin.hash() != hash {
                    return Err(InstallError::Unavailable);
                }
                Some(pin)
            }
            None => None,
        };
        let installed = self
            .world
            .install_program_with_provenance(
                module_hash.clone(),
                seed,
                Some(provenance.unwrap_or(crate::world::ModuleProvenance::Artifact)),
                guest,
            )
            .await
            .map_err(|_| InstallError::Unavailable)?;
        if !matches!(
            installed.kind,
            crate::world::WorldEventKind::ProgramInstalled { .. }
        ) {
            return Err(InstallError::Unavailable);
        }
        *self.module_pin.lock().await = pin;
        *self.module_hash.write().await = Some(module_hash.clone());
        Ok((module_hash, installed.sequence))
    }

    /// Read the currently pinned module artifact for peer download.
    /// The caller must already have authenticated as a world participant.
    pub async fn module_bytes(&self, expected_hash: &str) -> Result<Vec<u8>, InstallError> {
        let blobs = self.blobs.as_ref().ok_or(InstallError::Disabled)?;
        let pin = self.module_pin.lock().await;
        let Some(pin) = pin.as_ref() else {
            return Err(InstallError::Unavailable);
        };
        if pin.hash().to_string() != expected_hash {
            return Err(InstallError::InvalidUpload);
        }
        let bytes = blobs
            .blobs()
            .get_bytes(pin.hash())
            .await
            .map_err(|_| InstallError::Unavailable)?;
        if bytes.len() > MAX_WORLD_MODULE_BYTES || module_hash(&bytes) != expected_hash {
            return Err(InstallError::Unavailable);
        }
        Ok(bytes.to_vec())
    }

    /// Read a stored module's bytes by hash, active or not: from the blob
    /// store Wasm installs pin into, or from the module directory native
    /// installs are saved to. The caller must already have
    /// authenticated as an admin. A hash the host never installed is
    /// unavailable: the bytes of a module it does not hold cannot be
    /// conjured, and the error says so by kind.
    pub async fn stored_module_bytes(&self, expected_hash: &str) -> Result<Vec<u8>, InstallError> {
        let hash: iroh_blobs::Hash = expected_hash
            .parse()
            .map_err(|_| InstallError::InvalidUpload)?;
        // Wasm installs are pinned in the blob store; native installs are
        // saved only in the module directory (they are never offered for
        // peer download), so a stored native module is read from there.
        if let Some(blobs) = self.blobs.as_ref()
            && let Ok(bytes) = blobs.blobs().get_bytes(hash).await
            && bytes.len() <= MAX_WORLD_MODULE_BYTES
            && module_hash(&bytes) == expected_hash
        {
            return Ok(bytes.to_vec());
        }
        if let Some(modules) = self.module_dir.clone() {
            let requested = expected_hash.to_owned();
            let loaded = tokio::task::spawn_blocking(move || modules.load(&requested))
                .await
                .map_err(|_| InstallError::Unavailable)?
                .map_err(|_| InstallError::Unavailable)?;
            if loaded.len() <= MAX_WORLD_MODULE_BYTES {
                return Ok(loaded);
            }
        }
        if self.blobs.is_none() && self.module_dir.is_none() {
            return Err(InstallError::Disabled);
        }
        Err(InstallError::Unavailable)
    }

    /// Hash of the currently active module, if the world runs one.
    pub async fn active_module_hash(&self) -> Option<String> {
        self.module_hash.read().await.clone()
    }
}

/// Canonical content identifier expected in an install-begin frame.
#[must_use]
pub fn module_hash(wasm: &[u8]) -> String {
    Hash::new(wasm).to_string()
}

/// Constant-time (for equal lengths) secret comparison.
#[must_use]
pub fn secret_eq(expected: &[u8], supplied: &[u8]) -> bool {
    expected.len() == supplied.len() && bool::from(expected.ct_eq(supplied))
}

/// Run one session to completion with default settings.
pub async fn run_session<I: SessionIo>(hub: &WorldHub, io: &mut I) {
    run_session_with(hub, io, SessionConfig::default()).await;
}

/// The admin secret a first frame carries, if it is an administrative one.
/// A valid secret lets the frame create the world it names.
pub(crate) fn frame_admin_token(frame: &ClientFrame) -> Option<&str> {
    match frame {
        ClientFrame::Invite { admin_token, .. }
        | ClientFrame::InstallBegin { admin_token, .. }
        | ClientFrame::Compile { admin_token, .. }
        | ClientFrame::CreateWorld { admin_token }
        | ClientFrame::ListWorlds { admin_token } => Some(admin_token),
        ClientFrame::SetExecutionProfile { admin_token, .. } => Some(admin_token),
        _ => None,
    }
}

/// Run one session to completion.
///
/// The first frame names the world (`"world": "<name>"`, default world if
/// absent), and the session holds a lease on it until it ends.
pub async fn run_session_with<I: SessionIo>(hub: &WorldHub, io: &mut I, config: SessionConfig) {
    // Frames sent before a world lease exists belong to no world, so
    // their bytes go to a throwaway meter rather than being attributed
    // to a world that did not send them.
    let unattributed = WorldMetrics::default();
    let first = match tokio::time::timeout(config.join_timeout, io.recv()).await {
        Ok(Inbound::Text(text)) => text,
        Ok(Inbound::Oversized) => {
            let _ = send_error(&unattributed, io, "world frame is too large").await;
            return;
        }
        Ok(Inbound::Unsupported) => {
            let _ = send_error(&unattributed, io, "frames must be UTF-8 text").await;
            return;
        }
        Ok(Inbound::Closed) => return,
        Err(_) => {
            let _ = send_error(&unattributed, io, "timed out waiting for join").await;
            return;
        }
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&first) else {
        let _ = send_error(
            &unattributed,
            io,
            "first frame must be a valid join or invite request",
        )
        .await;
        return;
    };
    // `world` is a routing field, not part of any frame.
    let world = match value
        .as_object_mut()
        .and_then(|object| object.remove("world"))
    {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(name)) => Some(name),
        Some(_) => {
            let _ = send_error(&unattributed, io, "world must be a string").await;
            return;
        }
    };
    let Ok(frame) = serde_json::from_value::<ClientFrame>(value) else {
        let _ = send_error(
            &unattributed,
            io,
            "first frame must be a valid join or invite request",
        )
        .await;
        return;
    };
    // Hub-level frames are about the server, not one world.
    match &frame {
        ClientFrame::ListWorlds { admin_token } => {
            if admin_token.len() > MAX_ADMIN_TOKEN_BYTES || !hub.admin_ok(admin_token) {
                let _ = send_error(&unattributed, io, "admin denied").await;
                return;
            }
            io.mark_authorized();
            let worlds = hub.list();
            let _ = send_json(
                &unattributed,
                io,
                &ServerFrame::Worlds {
                    default: hub.default_world(),
                    worlds,
                },
            )
            .await;
            return;
        }
        ClientFrame::CreateWorld { admin_token } => {
            if admin_token.len() > MAX_ADMIN_TOKEN_BYTES || !hub.admin_ok(admin_token) {
                let _ = send_error(&unattributed, io, "admin denied").await;
                return;
            }
            io.mark_authorized();
            let name = world.as_deref().unwrap_or_else(|| hub.default_world());
            match hub.create(name).await {
                Ok(created) => {
                    let _ = send_json(
                        &unattributed,
                        io,
                        &ServerFrame::WorldCreated {
                            world: name,
                            created,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    let _ = send_error(&unattributed, io, error.message()).await;
                }
            }
            return;
        }
        _ => {}
    }
    let admin = frame_admin_token(&frame).filter(|token| token.len() <= MAX_ADMIN_TOKEN_BYTES);
    let lease = match hub.lease(world.as_deref(), admin).await {
        Ok(lease) => lease,
        Err(error) => {
            let _ = send_error(&unattributed, io, error.message()).await;
            return;
        }
    };
    if admin.is_some_and(|token| lease.service().authorize_admin(Some(token)).is_ok()) {
        io.mark_authorized();
    }
    if let ClientFrame::SetIsolation {
        isolation,
        admin_token,
        signed,
    } = &frame
    {
        match set_isolation_frame(
            hub,
            &lease,
            *isolation,
            admin_token.as_deref(),
            signed.as_ref(),
        )
        .await
        {
            Ok(isolation) => {
                io.mark_authorized();
                let _ = send_json(
                    lease.service().frame_meter(),
                    io,
                    &ServerFrame::Isolation {
                        world: lease.name(),
                        isolation,
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(lease.service().frame_meter(), io, &format!("{error:#}")).await;
            }
        }
        return;
    }
    if let Some(result) = policy_frame(hub, &lease, &frame).await {
        match result {
            Ok(policy) => {
                io.mark_authorized();
                let _ = send_json(
                    lease.service().frame_meter(),
                    io,
                    &ServerFrame::Policy {
                        world: lease.name(),
                        policy,
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(lease.service().frame_meter(), io, &format!("{error:#}")).await;
            }
        }
        return;
    }
    run_world_session(lease.service(), io, frame).await;
}

/// Apply an isolation change for the admin token, or for a signed request
/// whose account moderates the world.
pub(crate) async fn set_isolation_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    isolation: Isolation,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
) -> anyhow::Result<Isolation> {
    let target = format!("/world/{}/isolation", lease.name());
    let authority =
        moderation_authority(hub, lease, admin_token, signed, &target, isolation.as_str())?;
    lease
        .service()
        .world()
        .set_isolation(authority, isolation)
        .await
}

/// Handle a cross-world write or file scope frame; `None` for any other frame.
pub(crate) async fn policy_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    frame: &ClientFrame,
) -> Option<anyhow::Result<WorldPolicy>> {
    Some(match frame {
        ClientFrame::SetCrossWorldWrite {
            enabled,
            admin_token,
            signed,
        } => {
            set_cross_world_write_frame(
                hub,
                lease,
                *enabled,
                admin_token.as_deref(),
                signed.as_ref(),
            )
            .await
        }
        ClientFrame::ForceCrossWorldWrite { admin_token } => {
            force_cross_world_write_frame(lease, admin_token).await
        }
        ClientFrame::SetFileScope {
            scope,
            write_override,
            admin_token,
            signed,
        } => {
            set_file_scope_frame(
                hub,
                lease,
                *scope,
                *write_override,
                admin_token.as_deref(),
                signed.as_ref(),
            )
            .await
        }
        _ => return None,
    })
}

pub(crate) async fn set_cross_world_write_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    enabled: bool,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
) -> anyhow::Result<WorldPolicy> {
    let target = format!("/world/{}/cross-world-write", lease.name());
    let body = if enabled { "on" } else { "off" };
    let authority = moderation_authority(hub, lease, admin_token, signed, &target, body)?;
    lease
        .service()
        .world()
        .set_cross_world_write(authority, enabled)
        .await
}

pub(crate) async fn force_cross_world_write_frame(
    lease: &WorldLease,
    admin_token: &str,
) -> anyhow::Result<WorldPolicy> {
    anyhow::ensure!(is_admin(lease, Some(admin_token)), "admin denied");
    lease
        .service()
        .world()
        .force_cross_world_write(Authority::Admin)
        .await
}

pub(crate) async fn set_file_scope_frame(
    hub: &WorldHub,
    lease: &WorldLease,
    scope: FileScope,
    write_override: Option<bool>,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
) -> anyhow::Result<WorldPolicy> {
    let target = format!("/world/{}/file-scope", lease.name());
    let body = match scope {
        FileScope::Confined => "confined",
        FileScope::Unrestricted => "unrestricted",
    };
    let authority = moderation_authority(hub, lease, admin_token, signed, &target, body)?;
    lease
        .service()
        .world()
        .set_file_scope(authority, scope, write_override)
        .await
}

pub(crate) fn is_admin(lease: &WorldLease, admin_token: Option<&str>) -> bool {
    admin_token
        .filter(|token| token.len() <= MAX_ADMIN_TOKEN_BYTES)
        .is_some_and(|token| lease.service().authorize_admin(Some(token)).is_ok())
}

/// The admin when the token matches; otherwise a signed request's account,
/// which the world's moderator check then judges.
pub(crate) fn moderation_authority(
    hub: &WorldHub,
    lease: &WorldLease,
    admin_token: Option<&str>,
    signed: Option<&Signed>,
    target: &str,
    body: &str,
) -> anyhow::Result<Authority> {
    if is_admin(lease, admin_token) {
        return Ok(Authority::Admin);
    }
    let Some(signed) = signed else {
        anyhow::bail!("admin denied");
    };
    let key = hub.verify_signed(signed, "PUT", target, body.as_bytes())?;
    Ok(Authority::Signed(key))
}

/// Serve one session against its resolved world, starting from its first frame.
async fn run_world_session<I: SessionIo>(service: &WorldService, io: &mut I, frame: ClientFrame) {
    match frame {
        ClientFrame::Join {
            capability,
            after,
            screen_patches,
        } => {
            if capability.len() > MAX_CAPABILITY_BYTES {
                let _ = send_error(service.frame_meter(), io, "invalid join capability").await;
                return;
            }
            joined(
                service,
                io,
                JoinCapability::from_wire(capability),
                after,
                screen_patches,
            )
            .await;
        }
        ClientFrame::Invite {
            admin_token,
            display_name,
            expires_in_secs,
            uses,
            role,
        } => match role.as_deref().map(WorldScopes::from_role) {
            Some(None) => {
                let _ = send_error(service.frame_meter(),
                    io,
                    "unknown invite role; use spectator, observer, player, guest, delegator, builder, or moderator",
                )
                .await;
            }
            role_scopes => match service
                .invite_scoped(
                    Some(&admin_token),
                    display_name,
                    expires_in_secs,
                    uses,
                    role_scopes.flatten().unwrap_or(WorldScopes::GUEST),
                )
                .await
            {
                Ok(invite) => {
                    let _ =
                        send_json(service.frame_meter(), io, &ServerFrame::Invite(&invite)).await;
                }
                Err(InviteError::InvalidBounds(message)) => {
                    let _ = send_error(service.frame_meter(), io, message).await;
                }
                Err(InviteError::Disabled) => {
                    let _ = send_error(service.frame_meter(), io, "invites are not enabled").await;
                }
                Err(InviteError::Unauthorized) => {
                    let _ = send_error(service.frame_meter(), io, "invite denied").await;
                }
                Err(InviteError::Unavailable) => {
                    let _ = send_error(
                        service.frame_meter(),
                        io,
                        "world cannot issue another invite",
                    )
                    .await;
                }
            },
        },
        // Query frames keep the session open for further queries: a client
        // can page through records, and may still join later on.
        ClientFrame::Info { .. }
        | ClientFrame::Metrics { .. }
        | ClientFrame::Records { .. }
        | ClientFrame::RecordsTicket { .. }
        | ClientFrame::DownloadModule { .. } => {
            query_session(service, io, frame).await;
        }
        ClientFrame::InstallBegin {
            admin_token,
            total_bytes,
            module_hash,
            seed,
        } => {
            if service.source_only() {
                if service.authorize_admin(Some(&admin_token)).is_err() {
                    let _ = send_error(service.frame_meter(), io, "module install denied").await;
                    return;
                }
                let _ = send_error(
                    service.frame_meter(),
                    io,
                    "raw module uploads are disabled by source-only policy",
                )
                .await;
                return;
            }
            if let Err(message) = install_module_session(
                service,
                io,
                &admin_token,
                total_bytes,
                module_hash.as_deref(),
                seed,
            )
            .await
            {
                let _ = send_error(service.frame_meter(), io, message).await;
            }
        }
        ClientFrame::Caps { capability } => {
            let token = JoinCapability::from_wire(capability);
            if service.world().participant_id(token).await.is_err() {
                let _ = send_error(service.frame_meter(), io, "join denied").await;
            } else {
                io.mark_authorized();
                match service.world().caps_report().await {
                    Ok(report) => {
                        let _ = send_json(service.frame_meter(), io, &ServerFrame::Caps { report })
                            .await;
                    }
                    Err(_) => {
                        let _ =
                            send_error(service.frame_meter(), io, "capability report unavailable")
                                .await;
                    }
                }
            }
        }
        ClientFrame::UpdateCaps {
            admin_token,
            grant,
            revoke,
        } => match service.authorize_admin(Some(&admin_token)) {
            Ok(()) => match service.world().update_caps(grant, revoke).await {
                Ok(report) => {
                    let _ =
                        send_json(service.frame_meter(), io, &ServerFrame::Caps { report }).await;
                }
                Err(error) => {
                    let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            },
            Err(_) => {
                let _ = send_error(service.frame_meter(), io, "admin denied").await;
            }
        },
        ClientFrame::SetCapsCeiling {
            admin_token,
            ceiling,
        } => match service.set_caps_ceiling(Some(&admin_token), ceiling).await {
            Ok(report) => {
                let _ = send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::CapsCeiling {
                        report,
                        message: CAPS_CEILING_NOTICE,
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
            }
        },
        ClientFrame::SetExecutionProfile {
            admin_token,
            profile,
        } => match service
            .set_execution_profile(Some(&admin_token), profile)
            .await
        {
            Ok(update) => {
                let _ = send_json(service.frame_meter(),
                    io,
                    &ServerFrame::ExecutionProfile {
                        update,
                        message: "profile saved; it applies on the next program install or process restart, not to the running instance",
                    },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
            }
        },
        ClientFrame::SetSchedule {
            admin_token,
            id,
            at_ms,
        } => match service.set_schedule(Some(&admin_token), id, at_ms).await {
            Ok(schedules) => {
                let _ = send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::Schedules { schedules },
                )
                .await;
            }
            Err(error) => {
                let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
            }
        },
        ClientFrame::CancelSchedule { admin_token, id } => {
            match service.cancel_schedule(Some(&admin_token), id).await {
                Ok(schedules) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Schedules { schedules },
                    )
                    .await;
                }
                Err(error) => {
                    let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
        }
        ClientFrame::DownloadModuleAdmin {
            admin_token,
            module_hash,
        } => {
            if let Err(message) =
                download_module_admin_session(service, io, &admin_token, &module_hash).await
            {
                let _ = send_error(service.frame_meter(), io, message).await;
            }
        }
        ClientFrame::WorldState {
            admin_token,
            capability,
        } => {
            let admin_ok = admin_token
                .as_deref()
                .is_some_and(|token| service.authorize_admin(Some(token)).is_ok());
            let cap_ok = if admin_ok {
                true
            } else if let Some(cap) = capability.as_deref() {
                let token = JoinCapability::from_wire(cap.to_string());
                service.world().participant_id(token).await.is_ok()
            } else {
                false
            };
            if !admin_ok && !cap_ok {
                let _ = send_error(service.frame_meter(), io, "access denied").await;
            } else {
                match service.world().manifest_state().await {
                    Ok((seed, schedules)) => {
                        let _ = send_json(
                            service.frame_meter(),
                            io,
                            &ServerFrame::WorldState {
                                seed: admin_ok.then_some(seed),
                                schedules,
                            },
                        )
                        .await;
                    }
                    Err(error) => {
                        let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                    }
                }
            }
        }
        ClientFrame::ExecutionProfileInfo {
            admin_token,
            capability,
        } => {
            let auth_ok = if let Some(admin) = admin_token.as_deref() {
                service.authorize_admin(Some(admin)).is_ok()
            } else if let Some(cap) = capability.as_deref() {
                let token = JoinCapability::from_wire(cap.to_string());
                service.world().participant_id(token).await.is_ok()
            } else {
                false
            };
            if !auth_ok {
                let _ = send_error(service.frame_meter(), io, "access denied").await;
            } else {
                match service.world().execution_profile().await {
                    Ok(profile) => {
                        let _ = send_json(
                            service.frame_meter(),
                            io,
                            &ServerFrame::ExecutionProfile {
                                update: crate::world::ExecutionProfileUpdate {
                                    requested: profile,
                                    applies_on_next_program_install_or_process_restart: false,
                                },
                                message: "current persisted execution profile",
                            },
                        )
                        .await;
                    }
                    Err(error) => {
                        let _ = send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                    }
                }
            }
        }
        #[cfg(feature = "sandbox")]
        ClientFrame::Compile {
            admin_token,
            files,
            seed,
            native,
        } => {
            let seed = seed.unwrap_or_else(|| rand::rng().random::<u64>());
            let spec = CompileSpec {
                files: files
                    .into_iter()
                    .map(|file| (file.name, file.content))
                    .collect(),
                seed,
                native,
            };
            match service.compile_and_install(Some(&admin_token), &spec).await {
                Ok((module_hash, sequence, diagnostics)) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Compiled {
                            module_hash,
                            sequence,
                            diagnostics,
                        },
                    )
                    .await;
                }
                Err(message) => {
                    let _ = send_error(service.frame_meter(), io, &message).await;
                }
            }
        }
        #[cfg(not(feature = "sandbox"))]
        ClientFrame::Compile { .. } => {
            let _ = send_error(
                service.frame_meter(),
                io,
                "compilation is not enabled on this server",
            )
            .await;
        }
        ClientFrame::InstallChunk { .. }
        | ClientFrame::InstallEnd
        | ClientFrame::ListWorlds { .. }
        | ClientFrame::CreateWorld { .. }
        | ClientFrame::SetIsolation { .. }
        | ClientFrame::SetCrossWorldWrite { .. }
        | ClientFrame::ForceCrossWorldWrite { .. }
        | ClientFrame::SetFileScope { .. }
        | ClientFrame::Chat { .. }
        | ClientFrame::Attenuate { .. }
        | ClientFrame::DelegateToFriend { .. }
        | ClientFrame::Input { .. }
        | ClientFrame::Resize { .. }
        | ClientFrame::Paste { .. } => {
            let _ = send_error(
                service.frame_meter(),
                io,
                "first frame must be join, invite or install_begin",
            )
            .await;
        }
    }
}

/// Serve query frames (`info`, `records`, `records_ticket`,
/// `download_module`) until the peer closes, denies, or sends a frame that
/// ends the session. A `join` frame switches to the joined session.
async fn query_session<I: SessionIo>(service: &WorldService, io: &mut I, first: ClientFrame) {
    let mut next = Some(first);
    loop {
        let frame = match next.take() {
            Some(frame) => frame,
            None => match io.recv().await {
                Inbound::Text(text) => {
                    if let Ok(frame) = serde_json::from_str::<ClientFrame>(&text) {
                        frame
                    } else {
                        let _ = send_error(service.frame_meter(), io, "invalid world frame").await;
                        return;
                    }
                }
                Inbound::Oversized => {
                    let _ = send_error(service.frame_meter(), io, "world frame is too large").await;
                    return;
                }
                Inbound::Unsupported => {
                    let _ =
                        send_error(service.frame_meter(), io, "frames must be UTF-8 text").await;
                    return;
                }
                Inbound::Closed => return,
            },
        };
        match frame {
            ClientFrame::Join {
                capability,
                after,
                screen_patches,
            } => {
                if capability.len() > MAX_CAPABILITY_BYTES {
                    let _ = send_error(service.frame_meter(), io, "invalid join capability").await;
                    return;
                }
                joined(
                    service,
                    io,
                    JoinCapability::from_wire(capability),
                    after,
                    screen_patches,
                )
                .await;
                return;
            }
            ClientFrame::Info { capability } => {
                let token = JoinCapability::from_wire(capability);
                if let Ok(snapshot) = service.world().snapshot(token).await {
                    io.mark_authorized();
                    if !send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::ModuleInfo {
                            world_id: snapshot.world_id,
                            active_module_hash: snapshot.active_module_hash,
                            active_module_provenance: snapshot.active_module_provenance,
                            current_sequence: snapshot.current_sequence,
                        },
                    )
                    .await
                    {
                        return;
                    }
                } else {
                    let _ = send_error(service.frame_meter(), io, "join denied").await;
                    return;
                }
            }
            ClientFrame::Metrics { capability } => {
                // Authorized exactly like `info`: holding a capability
                // that snapshots the world is enough to read its metrics.
                let token = JoinCapability::from_wire(capability);
                match service.world().snapshot(token).await {
                    Ok(_) => {
                        io.mark_authorized();
                        match service.world().metrics().await {
                            Ok(metrics) => {
                                if !send_json(
                                    service.frame_meter(),
                                    io,
                                    &ServerFrame::Metrics { metrics },
                                )
                                .await
                                {
                                    return;
                                }
                            }
                            Err(error) => {
                                let _ =
                                    send_error(service.frame_meter(), io, &format!("{error:#}"))
                                        .await;
                                return;
                            }
                        }
                    }
                    Err(_) => {
                        let _ = send_error(service.frame_meter(), io, "join denied").await;
                        return;
                    }
                }
            }
            ClientFrame::Records {
                capability,
                prefix,
                after,
            } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token.clone()).await.is_ok() {
                    io.mark_authorized();
                }
                if let Err(message) = records_session(
                    service.frame_meter(),
                    service.world(),
                    io,
                    token,
                    prefix.as_deref(),
                    after.as_deref(),
                )
                .await
                {
                    let _ = send_error(service.frame_meter(), io, message).await;
                    return;
                }
            }
            ClientFrame::RecordsTicket { capability } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().participant_id(token).await.is_err() {
                    let _ = send_error(service.frame_meter(), io, "join denied").await;
                    return;
                }
                io.mark_authorized();
                match records_ticket_session(service, io).await {
                    Ok(()) => {}
                    Err(message) => {
                        let _ = send_error(service.frame_meter(), io, message).await;
                        return;
                    }
                }
            }
            ClientFrame::DownloadModule {
                capability,
                module_hash,
            } => {
                let token = JoinCapability::from_wire(capability);
                if service.world().snapshot(token).await.is_err() {
                    let _ = send_error(service.frame_meter(), io, "join denied").await;
                    return;
                }
                io.mark_authorized();
                if let Err(message) = download_module_session(service, io, &module_hash).await {
                    let _ = send_error(service.frame_meter(), io, message).await;
                    return;
                }
            }
            _ => {
                let _ = send_error(service.frame_meter(), io, "expected a query frame").await;
                return;
            }
        }
    }
}

/// Answer a `records_ticket` query.
async fn records_ticket_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
) -> Result<(), &'static str> {
    match service.records_store() {
        Some(store) => match store.read_ticket().await {
            Ok(ticket) => {
                if send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::RecordsTicket { ticket },
                )
                .await
                {
                    Ok(())
                } else {
                    Err("connection closed")
                }
            }
            Err(_) => Err("records ticket unavailable"),
        },
        None => Err("world keeps no structured records"),
    }
}

/// Largest page of records returned in one frame, comfortably inside
/// [`MAX_FRAME_BYTES`] after JSON framing overhead.
pub(crate) const MAX_RECORDS_PAGE_BYTES: usize = 768 * 1024;

async fn records_session<I: SessionIo>(
    meter: &WorldMetrics,
    world: &WorldHandle,
    io: &mut I,
    token: JoinCapability,
    prefix: Option<&str>,
    after: Option<&str>,
) -> Result<(), &'static str> {
    if world.participant_id(token).await.is_err() {
        return Err("join denied");
    }
    let (sequence, records) = world.records();
    let mut page = crate::world::Records::new();
    let mut used = 0usize;
    let mut last_included: Option<String> = None;
    let mut more = false;
    for (key, value) in records.iter() {
        if let Some(prefix) = prefix
            && !key.starts_with(prefix)
        {
            continue;
        }
        if let Some(after) = after
            && key.as_str() <= after
        {
            continue;
        }
        let size = key.len() + serde_json::to_vec(value).map_or(0, |v| v.len()) + 8;
        if used + size > MAX_RECORDS_PAGE_BYTES {
            // `next_after` is exclusive on the client, so it must be the
            // last key *included* here; anything else would skip a record.
            if page.is_empty() {
                return Err("a record is too large to serve in one page");
            }
            more = true;
            break;
        }
        used += size;
        last_included = Some(key.clone());
        page.insert(key.clone(), value.clone());
    }
    let next_after = more.then_some(last_included).flatten();
    if !send_json(
        meter,
        io,
        &ServerFrame::Records {
            sequence,
            records: page,
            next_after,
        },
    )
    .await
    {
        return Err("connection closed");
    }
    Ok(())
}

async fn download_module_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    requested_hash: &str,
) -> Result<(), &'static str> {
    let Some(active_hash) = service.active_module_hash().await else {
        return Err("world has no installed module");
    };
    if requested_hash != active_hash {
        return Err("requested module is not the active world module");
    }
    let bytes = service
        .module_bytes(&active_hash)
        .await
        .map_err(|_| "module bytes are unavailable")?;
    send_module_bytes(service, io, active_hash, &bytes).await
}

/// The admin module download (#807): any module the host holds in its blob
/// store, by hash, active or not - so `id world apply` can install the
/// module a manifest names when another module is currently active.
async fn download_module_admin_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    admin_token: &str,
    requested_hash: &str,
) -> Result<(), &'static str> {
    if admin_token.len() > MAX_ADMIN_TOKEN_BYTES
        || service.authorize_admin(Some(admin_token)).is_err()
    {
        return Err("admin denied");
    }
    let bytes = service
        .stored_module_bytes(requested_hash)
        .await
        .map_err(|_| "the host does not hold a module with that hash")?;
    send_module_bytes(service, io, requested_hash.to_owned(), &bytes).await
}

/// Stream one module's bytes as a begin/chunk/end transfer naming `module_hash`.
async fn send_module_bytes<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    module_hash: String,
    bytes: &[u8],
) -> Result<(), &'static str> {
    if !send_json(
        service.frame_meter(),
        io,
        &ServerFrame::ModuleBegin {
            module_hash: module_hash.clone(),
            total_bytes: bytes.len(),
        },
    )
    .await
    {
        return Err("connection closed");
    }
    for (index, chunk) in bytes.chunks(MAX_WORLD_MODULE_CHUNK_BYTES).enumerate() {
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::ModuleChunk {
                offset: index * MAX_WORLD_MODULE_CHUNK_BYTES,
                data_hex: encode_hex(chunk),
            },
        )
        .await
        {
            return Err("connection closed");
        }
    }
    if !send_json(
        service.frame_meter(),
        io,
        &ServerFrame::ModuleEnd { module_hash },
    )
    .await
    {
        return Err("connection closed");
    }
    Ok(())
}

async fn install_module_session<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    admin_token: &str,
    total_bytes: usize,
    claimed_hash: Option<&str>,
    seed: u64,
) -> Result<(), &'static str> {
    if admin_token.len() > MAX_ADMIN_TOKEN_BYTES
        || service.authorize_admin(Some(admin_token)).is_err()
    {
        return Err("module install denied");
    }
    if total_bytes == 0
        || total_bytes > MAX_WORLD_MODULE_BYTES
        || claimed_hash.is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err("invalid module size or content hash");
    }
    let upload_permit = service
        .try_reserve_module_upload()
        .map_err(|_| "world is busy installing another module")?;
    #[cfg(not(feature = "sandbox"))]
    {
        let _ = (io, seed, upload_permit);
        return Err("module installation requires the sandbox feature");
    }

    #[cfg(feature = "sandbox")]
    {
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::UploadReady {
                chunk_bytes: MAX_WORLD_MODULE_CHUNK_BYTES,
            },
        )
        .await
        {
            return Err("connection closed");
        }
        let mut bytes = Vec::with_capacity(total_bytes);
        while bytes.len() < total_bytes {
            let incoming = tokio::time::timeout(Duration::from_secs(30), io.recv())
                .await
                .map_err(|_| "timed out receiving module")?;
            let Inbound::Text(text) = incoming else {
                return Err("invalid module chunk frame");
            };
            let Ok(ClientFrame::InstallChunk { offset, data_hex }) =
                serde_json::from_str::<ClientFrame>(&text)
            else {
                return Err("expected module chunk frame");
            };
            if offset != bytes.len() || data_hex.len() > MAX_WORLD_MODULE_CHUNK_BYTES * 2 {
                return Err("invalid module chunk offset or size");
            }
            let Some(chunk) = decode_hex(&data_hex) else {
                return Err("module chunk must be hexadecimal");
            };
            if chunk.is_empty() || chunk.len() > total_bytes.saturating_sub(bytes.len()) {
                return Err("invalid module chunk length");
            }
            bytes.extend_from_slice(&chunk);
        }

        let incoming = tokio::time::timeout(Duration::from_secs(30), io.recv())
            .await
            .map_err(|_| "timed out waiting for install completion")?;
        if !matches!(incoming, Inbound::Text(ref text) if matches!(serde_json::from_str::<ClientFrame>(text), Ok(ClientFrame::InstallEnd)))
        {
            return Err("expected install_end frame");
        }
        let actual_hash = module_hash(&bytes);
        if claimed_hash.is_some_and(|claimed| actual_hash != claimed.to_ascii_lowercase()) {
            return Err("module content hash mismatch");
        }
        let (module_hash, sequence) = service
            .install_wasm_reserved(
                Some(admin_token),
                bytes,
                seed,
                &actual_hash,
                upload_permit,
                false,
                None,
            )
            .await
            .map_err(|error| match error {
                InstallError::Disabled => "module installation is not enabled on this host",
                InstallError::Unauthorized => "module install denied",
                InstallError::InvalidUpload => "invalid module upload",
                InstallError::RawModuleDisabled => {
                    "raw module uploads are disabled by source-only policy"
                }
                InstallError::InvalidModule => "Wasm module failed validation or initialization",
                InstallError::Unavailable => "world could not activate the module",
            })?;
        if !send_json(
            service.frame_meter(),
            io,
            &ServerFrame::ModuleInstalled {
                module_hash: &module_hash,
                sequence,
            },
        )
        .await
        {
            return Err("connection closed");
        }
        Ok(())
    }
}

async fn joined<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: JoinCapability,
    after: Option<u64>,
    screen_patches: bool,
) {
    let world = service.world();
    // Subscribe before taking the snapshot so no event can fall in the gap;
    // events at or below the cursor are skipped when delivered.
    let mut events = world.subscribe();
    let mut revocations = world.subscribe_revocations();
    let Ok(participant_id) = world.participant_id(token.clone()).await else {
        let _ = send_error(service.frame_meter(), io, "join denied").await;
        return;
    };
    let Ok(snapshot) = world.snapshot(token.clone()).await else {
        let _ = send_error(service.frame_meter(), io, "join denied").await;
        return;
    };
    io.mark_authorized();
    let mut cursor = snapshot.current_sequence;
    let mut viewport = None;
    let mut view_state = ViewRevisionState::default();
    if let Some(after) = after {
        match world.events_after(token.clone(), after).await {
            Ok(page) if page.needs_snapshot => {
                let Ok(fresh) = world.snapshot(token.clone()).await else {
                    return;
                };
                cursor = fresh.current_sequence;
                if !send_json(
                    service.frame_meter(),
                    io,
                    &ServerFrame::Snapshot { snapshot: &fresh },
                )
                .await
                {
                    return;
                }
            }
            Ok(page) => {
                cursor = page.current_sequence;
                for event in &page.events {
                    if !send_json(service.frame_meter(), io, &ServerFrame::Event { event }).await {
                        return;
                    }
                    if !send_view(
                        service.frame_meter(),
                        world,
                        io,
                        &token,
                        event.sequence,
                        viewport,
                        &mut view_state,
                        screen_patches,
                    )
                    .await
                    {
                        return;
                    }
                }
            }
            Err(_) => {
                let _ = send_error(service.frame_meter(), io, "join denied").await;
                return;
            }
        }
    } else if !send_json(
        service.frame_meter(),
        io,
        &ServerFrame::Snapshot {
            snapshot: &snapshot,
        },
    )
    .await
    {
        return;
    }
    if !send_view(
        service.frame_meter(),
        world,
        io,
        &token,
        cursor,
        viewport,
        &mut view_state,
        screen_patches,
    )
    .await
    {
        return;
    }

    // Presence starts here and ends whenever this session does. Views are
    // re-sent whenever program state changes for another reason (host events
    // are not broadcast as events).
    world.presence(participant_id, true).await;
    let _guard = PresenceGuard {
        world: world.clone(),
        participant: participant_id,
    };
    let mut views = world.watch_views();
    let mut pending_view_deadline = None;
    let mut pending_view_sequence = cursor;

    loop {
        let view_deadline = pending_view_deadline;
        tokio::select! {
            () = async {
                match view_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                pending_view_deadline = None;
                if !send_view(service.frame_meter(), world, io, &token, pending_view_sequence, viewport, &mut view_state, screen_patches).await {
                    break;
                }
            },
            inbound = io.recv() => match inbound {
                Inbound::Text(text) => {
                    if !handle_client_frame(
                        service,
                        io,
                        &token,
                        &text,
                        &mut viewport,
                        cursor,
                        &mut view_state,
                        screen_patches,
                        &mut pending_view_deadline,
                    ).await {
                        break;
                    }
                }
                Inbound::Oversized => {
                    if !send_error(service.frame_meter(), io, "world frame is too large").await { break; }
                }
                Inbound::Unsupported => {
                    if !send_error(service.frame_meter(), io, "frames must be UTF-8 text").await { break; }
                }
                Inbound::Closed => break,
            },
            event = events.recv() => match event {
                Ok(event) => {
                    if let crate::world::WorldEventKind::ProgramInstalled { ref module_hash, .. } = event.kind {
                        if event.sequence > cursor {
                            pending_view_deadline = None;
                            let _ = views.borrow_and_update();
                            let Ok(snapshot) = world.snapshot(token.clone()).await else { break };
                            if !send_json(service.frame_meter(), io, &ServerFrame::Snapshot { snapshot: &snapshot }).await {
                                break;
                            }
                            cursor = event.sequence;
                            if !send_json(service.frame_meter(), io, &ServerFrame::ProgramChanged {
                                module_hash,
                                sequence: cursor,
                            }).await {
                                break;
                            }
                            view_state.reset();
                            if !send_view(service.frame_meter(), world, io, &token, cursor, viewport, &mut view_state, screen_patches).await {
                                break;
                            }
                        }
                        continue;
                    }
                    if event.sequence <= cursor {
                        continue;
                    }
                    // Projecting the event re-validates the capability on every
                    // delivery, which is what disconnects a revoked
                    // participant, and applies what this viewer may see now.
                    let Ok(projected) = world.project_event(token.clone(), event.clone()).await else {
                        break;
                    };
                    // A withheld event still moves the cursor past it.
                    cursor = event.sequence;
                    let Some(shown) = projected else {
                        continue;
                    };
                    if !send_json(service.frame_meter(), io, &ServerFrame::Event { event: &shown }).await {
                        break;
                    }
                    pending_view_sequence = cursor;
                    pending_view_deadline.get_or_insert_with(|| tokio::time::Instant::now() + VIEW_COALESCE_WINDOW);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    pending_view_deadline = None;
                    let Ok(snapshot) = world.snapshot(token.clone()).await else { break };
                    let _ = views.borrow_and_update();
                    if !send_json(service.frame_meter(), io, &ServerFrame::Snapshot { snapshot: &snapshot }).await {
                        break;
                    }
                    cursor = snapshot.current_sequence;
                    view_state.reset();
                    if !send_view(service.frame_meter(), world, io, &token, cursor, viewport, &mut view_state, screen_patches).await {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            changed = views.changed() => match changed {
                Ok(()) => {
                    let sequence = *views.borrow_and_update();
                    pending_view_sequence = sequence;
                    pending_view_deadline.get_or_insert_with(|| tokio::time::Instant::now() + VIEW_COALESCE_WINDOW);
                }
                Err(_) => break,
            },
            revoked = revocations.recv() => match revoked {
                Ok(revoked_id) if revoked_id == participant_id => {
                    let _ = send_error(service.frame_meter(), io, "world capability revoked").await;
                    break;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {},
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }
}

/// Returns the participant's presence when the session ends, however it ends.
pub(crate) struct PresenceGuard {
    world: WorldHandle,
    participant: u64,
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        let world = self.world.clone();
        let participant = self.participant;
        tokio::spawn(async move {
            world.presence(participant, false).await;
        });
    }
}

#[derive(Default)]
struct ViewRevisionState {
    next_revision: u64,
    screen: Option<(u64, crate::world_presentation::ScreenSurface)>,
}

impl ViewRevisionState {
    fn reset(&mut self) {
        self.screen = None;
    }
}

async fn send_view<I: SessionIo>(
    meter: &WorldMetrics,
    world: &WorldHandle,
    io: &mut I,
    token: &JoinCapability,
    sequence: u64,
    viewport: Option<Viewport>,
    view_state: &mut ViewRevisionState,
    screen_patches: bool,
) -> bool {
    let view = match viewport {
        Some(viewport) => {
            let width = u16::try_from(viewport.width).unwrap_or(u16::MAX);
            let height = u16::try_from(viewport.height).unwrap_or(u16::MAX);
            world.view_at(token.clone(), width, height).await
        }
        None => world.view(token.clone()).await,
    };
    match view {
        Ok(Some(data)) => {
            if data.len() > MAX_FRAME_BYTES {
                return send_error(meter, io, "world presentation is too large").await;
            }
            let surface = match crate::world_presentation::parse_surface(&data) {
                Ok(Some(crate::world_presentation::WorldSurface::Screen(screen))) => {
                    let screen = viewport.map_or(screen.clone(), |viewport| {
                        screen.fit(viewport.width, viewport.height)
                    });
                    Some(crate::world_presentation::WorldSurface::Screen(screen))
                }
                Ok(surface) => surface,
                Err(_) => return send_error(meter, io, "invalid world screen presentation").await,
            };
            let view_revision = view_state.next_revision;
            view_state.next_revision = view_state.next_revision.saturating_add(1);
            let screen = match surface.as_ref() {
                Some(crate::world_presentation::WorldSurface::Screen(screen)) => Some(screen),
                _ => None,
            };

            if let (true, Some(screen), Some((base_view_revision, previous))) =
                (screen_patches, screen, view_state.screen.as_ref())
            {
                if let Some(patch) = crate::world_presentation::ScreenPatch::diff(previous, screen)
                {
                    let delta_frame = ServerFrame::View {
                        sequence,
                        data: "",
                        surface: None,
                        view_revision,
                        base_view_revision: Some(*base_view_revision),
                        patch: Some(&patch),
                    };
                    let full_frame = ServerFrame::View {
                        sequence,
                        data: &data,
                        surface: surface.as_ref(),
                        view_revision,
                        base_view_revision: None,
                        patch: None,
                    };
                    let delta_size = serde_json::to_vec(&delta_frame).map(|bytes| bytes.len());
                    let full_size = serde_json::to_vec(&full_frame).map(|bytes| bytes.len());
                    if matches!((delta_size, full_size), (Ok(delta), Ok(full)) if delta < full) {
                        let sent = send_json(meter, io, &delta_frame).await;
                        if sent {
                            view_state.screen = Some((view_revision, screen.clone()));
                        }
                        return sent;
                    }
                }
            }

            let full_frame = ServerFrame::View {
                sequence,
                data: &data,
                surface: surface.as_ref(),
                view_revision,
                base_view_revision: None,
                patch: None,
            };
            let sent = send_json(meter, io, &full_frame).await;
            if sent && screen_patches {
                view_state.screen = screen.map(|screen| (view_revision, screen.clone()));
            } else {
                view_state.reset();
            }
            sent
        }
        Ok(None) => {
            view_state.reset();
            true
        }
        Err(_) => false,
    }
}

/// Connection-local terminal/browser dimensions. This value is intentionally
/// absent from world events and snapshots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Viewport {
    width: usize,
    height: usize,
}

pub(crate) fn normalize_paste(text: &str) -> Result<Vec<u8>, &'static str> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    if normalized.len() > 4096
        || normalized
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return Err("paste must be at most 4096 UTF-8 bytes and contain no control characters");
    }
    Ok(normalized.into_bytes())
}

const USES_REFUSED: &str = "a delegation may not carry `uses`: a delegated grant has no use count, and the host's limit still applies";

/// Whether a delegation frame names `uses` anywhere in it. The field was
/// removed from delegations, and serde would ignore it, minting an unbounded
/// grant from a client that still sends it. Any depth is refused, so no
/// position hides the field.
pub(crate) fn delegation_names_uses(value: &serde_json::Value) -> bool {
    let delegating = matches!(
        value.get("type").and_then(serde_json::Value::as_str),
        Some("attenuate" | "delegate_to_friend")
    );
    delegating && names_uses(value)
}

pub(crate) fn names_uses(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .any(|(key, inner)| key == "uses" || names_uses(inner)),
        serde_json::Value::Array(items) => items.iter().any(names_uses),
        _ => false,
    }
}

/// Handle one frame from a joined participant. Returns `false` to end the session.
async fn handle_client_frame<I: SessionIo>(
    service: &WorldService,
    io: &mut I,
    token: &JoinCapability,
    text: &str,
    viewport: &mut Option<Viewport>,
    sequence: u64,
    view_state: &mut ViewRevisionState,
    screen_patches: bool,
    pending_view_deadline: &mut Option<tokio::time::Instant>,
) -> bool {
    let world = service.world();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return send_error(service.frame_meter(), io, "invalid world frame").await;
    };
    if delegation_names_uses(&value) {
        return send_error(service.frame_meter(), io, USES_REFUSED).await;
    }
    let Ok(frame) = serde_json::from_value::<ClientFrame>(value) else {
        return send_error(service.frame_meter(), io, "invalid world frame").await;
    };
    let result = match frame {
        ClientFrame::Attenuate {
            name,
            scopes,
            expires_at,
        } => {
            let Some(scopes) = WorldScopes::from_names(scopes.iter().map(String::as_str)) else {
                return send_error(service.frame_meter(), io, "unknown scope").await;
            };
            let bounds = CapabilityBounds {
                expires_at,
                uses: None,
            };
            let result = world.attenuate(token.clone(), scopes, bounds, name).await;
            return send_delegation(service.frame_meter(), io, result).await;
        }
        ClientFrame::DelegateToFriend {
            friend,
            name,
            scopes,
            expires_at,
        } => {
            let Some(scopes) = WorldScopes::from_names(scopes.iter().map(String::as_str)) else {
                return send_error(service.frame_meter(), io, "unknown scope").await;
            };
            let bounds = CapabilityBounds {
                expires_at,
                uses: None,
            };
            let result = world
                .delegate_to_friend(token.clone(), friend, scopes, bounds, name)
                .await;
            return send_delegation(service.frame_meter(), io, result).await;
        }
        ClientFrame::Chat { text } => world.chat(token.clone(), text).await,
        ClientFrame::Resize { width, height } => {
            if width == 0 || height == 0 || width > 500 || height > 200 {
                return send_error(
                    service.frame_meter(),
                    io,
                    "viewport dimensions must be width 1..=500 and height 1..=200",
                )
                .await;
            }
            let clamped_width = usize::from(width).min(crate::world_presentation::MAX_SCREEN_WIDTH);
            let clamped_height =
                usize::from(height).min(crate::world_presentation::MAX_SCREEN_HEIGHT);
            *viewport = Some(Viewport {
                width: clamped_width,
                height: clamped_height,
            });
            // The immediate viewport-specific view includes the latest state,
            // so it supersedes any coalesced redraw already pending.
            *pending_view_deadline = None;
            view_state.reset();
            return send_view(
                service.frame_meter(),
                world,
                io,
                token,
                sequence,
                *viewport,
                view_state,
                screen_patches,
            )
            .await;
        }
        ClientFrame::Paste { text } => {
            let Ok(bytes) = normalize_paste(&text) else {
                return send_error(
                    service.frame_meter(),
                    io,
                    "paste must be at most 4096 UTF-8 bytes and contain no control characters",
                )
                .await;
            };
            world.input(token.clone(), bytes).await
        }
        ClientFrame::Input { data_hex } => match decode_hex(&data_hex) {
            Some(data) => world.input(token.clone(), data).await,
            None => {
                return send_error(service.frame_meter(), io, "input data must be hexadecimal")
                    .await;
            }
        },
        ClientFrame::Records { prefix, after, .. } => {
            // Authorized by the session's own capability.
            if let Err(message) = records_session(
                service.frame_meter(),
                world,
                io,
                token.clone(),
                prefix.as_deref(),
                after.as_deref(),
            )
            .await
            {
                return send_error(service.frame_meter(), io, message).await;
            }
            return true;
        }
        ClientFrame::SetExecutionProfile {
            admin_token,
            profile,
        } => {
            match service
                .set_execution_profile(Some(&admin_token), profile)
                .await
            {
                Ok(update) => {
                    let _ = send_json(service.frame_meter(),
                        io,
                        &ServerFrame::ExecutionProfile {
                            update,
                            message: "profile saved; it applies on the next program install or process restart, not to the running instance",
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::ExecutionProfileInfo { .. } => {
            match service.world().execution_profile().await {
                Ok(profile) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::ExecutionProfile {
                            update: crate::world::ExecutionProfileUpdate {
                                requested: profile,
                                applies_on_next_program_install_or_process_restart: false,
                            },
                            message: "current persisted execution profile",
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::RecordsTicket { .. } => {
            if let Err(message) = records_ticket_session(service, io).await {
                return send_error(service.frame_meter(), io, message).await;
            }
            return true;
        }
        // Authorized by the session's own capability, like `records`; the
        // shell contract lists `info` in the joined phase (`/stats`).
        ClientFrame::Info { .. } => {
            let Ok(snapshot) = world.snapshot(token.clone()).await else {
                return send_error(service.frame_meter(), io, "join denied").await;
            };
            return send_json(
                service.frame_meter(),
                io,
                &ServerFrame::ModuleInfo {
                    world_id: snapshot.world_id,
                    active_module_hash: snapshot.active_module_hash,
                    active_module_provenance: snapshot.active_module_provenance,
                    current_sequence: snapshot.current_sequence,
                },
            )
            .await;
        }
        // Like `info`, the joined phase may read the world's metrics with
        // the session's own capability — gated on a snapshot with the
        // session token first, so a revoked participant is refused here
        // even when this frame wins the race against the revocation
        // broadcast (#1506).
        ClientFrame::Metrics { .. } => {
            if world.snapshot(token.clone()).await.is_err() {
                return send_error(service.frame_meter(), io, "join denied").await;
            }
            match world.metrics().await {
                Ok(metrics) => {
                    return send_json(service.frame_meter(), io, &ServerFrame::Metrics { metrics })
                        .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
        }
        ClientFrame::Caps { .. } => {
            // Same capability gate as the query phase (#1506): the
            // session token must still name a live participant.
            if world.participant_id(token.clone()).await.is_err() {
                return send_error(service.frame_meter(), io, "join denied").await;
            }
            match service.world().caps_report().await {
                Ok(report) => {
                    let _ =
                        send_json(service.frame_meter(), io, &ServerFrame::Caps { report }).await;
                }
                Err(_) => {
                    return send_error(service.frame_meter(), io, "capability report unavailable")
                        .await;
                }
            }
            return true;
        }
        ClientFrame::UpdateCaps {
            admin_token,
            grant,
            revoke,
        } => {
            match service.authorize_admin(Some(&admin_token)) {
                Ok(()) => match service.world().update_caps(grant, revoke).await {
                    Ok(report) => {
                        let _ = send_json(service.frame_meter(), io, &ServerFrame::Caps { report })
                            .await;
                    }
                    Err(error) => {
                        return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                    }
                },
                Err(_) => {
                    return send_error(service.frame_meter(), io, "admin denied").await;
                }
            }
            return true;
        }
        ClientFrame::SetCapsCeiling {
            admin_token,
            ceiling,
        } => {
            match service.set_caps_ceiling(Some(&admin_token), ceiling).await {
                Ok(report) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::CapsCeiling {
                            report,
                            message: CAPS_CEILING_NOTICE,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::SetSchedule {
            admin_token,
            id,
            at_ms,
        } => {
            match service.set_schedule(Some(&admin_token), id, at_ms).await {
                Ok(schedules) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Schedules { schedules },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::CancelSchedule { admin_token, id } => {
            match service.cancel_schedule(Some(&admin_token), id).await {
                Ok(schedules) => {
                    let _ = send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::Schedules { schedules },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
            return true;
        }
        ClientFrame::WorldState { admin_token, .. } => {
            let admin_ok = admin_token
                .as_deref()
                .is_some_and(|token| service.authorize_admin(Some(token)).is_ok());
            // The session's own capability authorizes the read, like `info`;
            // only a valid admin token in the frame unlocks the seed (#240).
            match world.manifest_state().await {
                Ok((seed, schedules)) => {
                    return send_json(
                        service.frame_meter(),
                        io,
                        &ServerFrame::WorldState {
                            seed: admin_ok.then_some(seed),
                            schedules,
                        },
                    )
                    .await;
                }
                Err(error) => {
                    return send_error(service.frame_meter(), io, &format!("{error:#}")).await;
                }
            }
        }
        ClientFrame::Join { .. }
        | ClientFrame::Invite { .. }
        | ClientFrame::ListWorlds { .. }
        | ClientFrame::CreateWorld { .. }
        | ClientFrame::SetIsolation { .. }
        | ClientFrame::SetCrossWorldWrite { .. }
        | ClientFrame::ForceCrossWorldWrite { .. }
        | ClientFrame::SetFileScope { .. }
        | ClientFrame::Compile { .. }
        | ClientFrame::DownloadModule { .. }
        | ClientFrame::DownloadModuleAdmin { .. }
        | ClientFrame::InstallBegin { .. }
        | ClientFrame::InstallChunk { .. }
        | ClientFrame::InstallEnd => {
            return send_error(service.frame_meter(), io, "already joined").await;
        }
    };
    // Accepted events reach everyone, including the sender, through the
    // broadcast branch, so only rejections are answered directly.
    result.is_ok() || send_error(service.frame_meter(), io, "world event rejected").await
}

/// Serialize and send one server frame, metering its bytes into the
/// world's host metrics (F11.7 frame bytes).
async fn send_json<I: SessionIo>(meter: &WorldMetrics, io: &mut I, frame: &impl Serialize) -> bool {
    match serde_json::to_string(frame) {
        Ok(encoded) if encoded.len() <= MAX_FRAME_BYTES => {
            let bytes = encoded.len() as u64;
            let sent = io.send(encoded).await;
            if sent {
                meter.add_frame_bytes(bytes);
            }
            sent
        }
        _ => false,
    }
}

async fn send_error<I: SessionIo>(meter: &WorldMetrics, io: &mut I, message: &str) -> bool {
    send_json(meter, io, &ServerFrame::Error { message }).await
}

async fn send_delegation<I: SessionIo>(
    meter: &WorldMetrics,
    io: &mut I,
    result: anyhow::Result<(Participant, JoinCapability)>,
) -> bool {
    match result {
        Ok((participant, capability)) => {
            let delegated = InviteResponse {
                capability: capability.expose().to_owned(),
                participant_id: participant.id,
                display_name: participant.display_name,
            };
            send_json(meter, io, &ServerFrame::Invite(&delegated)).await
        }
        Err(_) => send_error(meter, io, "delegation denied").await,
    }
}

/// Decode a bounded hex string.
#[must_use]
pub fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) || encoded.len() > MAX_FRAME_BYTES * 2 {
        return None;
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?))
        .collect()
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    result
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn enqueue_all(
    queue: &mut crate::envelope_outbox::Outbox,
    outbound: Vec<crate::directory_view::Outbound>,
) -> anyhow::Result<Vec<String>> {
    use crate::envelope_outbox::Payload;
    let mut keys = Vec::with_capacity(outbound.len());
    for crate::directory_view::Outbound { target, payload } in outbound {
        let key = payload.key(&target);
        let queued = match payload {
            Payload::Envelope(envelope) => queue.enqueue(&target, envelope),
            Payload::Artifact(artifact) => queue.enqueue_artifact(&target, artifact),
        };
        if let Err(error) = queued {
            withdraw_all(queue, &keys);
            return Err(error);
        }
        keys.push(key);
    }
    Ok(keys)
}

pub(crate) fn withdraw_all(queue: &mut crate::envelope_outbox::Outbox, keys: &[String]) {
    for key in keys {
        if let Err(withdrawn) = queue.withdraw(key) {
            tracing::error!("{key} stays queued though its change was refused: {withdrawn:#}");
        }
    }
}
