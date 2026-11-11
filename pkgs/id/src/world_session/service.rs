//! World-service constructors and module lifecycle.
//!
//! Split from `mod.rs` (#4717): the `WorldService` implementation lived in
//! the session-driver module and pushed it over the F11.10 2,500-line
//! production limit. The type itself stays in `types.rs`; this module only
//! hosts the `impl` block so `mod.rs` keeps the wire-protocol driver. No
//! behavior or visibility change: methods resolve through the same
//! `crate::world_session::WorldService` path.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Context as _;
use iroh_blobs::{
    BlobFormat, Hash,
    api::{Store, blobs::AddBytesOptions},
};
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};

use crate::time::unix_ms;
use crate::world::{JoinCapability, SessionId, WorldHandle, WorldMetrics, WorldScopes};
use crate::world_compile::Compiler;
#[cfg(feature = "sandbox")]
use crate::world_compile::{COMPILE_TIMEOUT, CompileSpec};

use super::types::{
    InstallError, InviteError, InviteResponse, MAX_ADMIN_TOKEN_BYTES, MAX_WORLD_MODULE_BYTES,
    ModuleSignature, WorldService,
};
use super::{
    MAX_CONCURRENT_MODULE_UPLOADS, enqueue_all, invite_bounds, module_hash, secret_eq,
    verify_module_signature, withdraw_all,
};

impl WorldService {
    /// Allocate an ephemeral identity for one accepted connection.
    pub(crate) fn next_session_id(&self) -> SessionId {
        SessionId::new(self.next_session_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Require module artifacts to carry a valid signature from one of these
    /// Ed25519 public keys. An empty list preserves the legacy unsigned path.
    #[must_use]
    pub fn with_module_trust_keys(mut self, keys: Vec<[u8; 32]>) -> Self {
        self.trusted_module_keys = keys;
        self
    }

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
            trusted_module_keys: Vec::new(),
            runtime: crate::world_limits::RuntimeLimits::default(),
            mail: None,
            outbox: None,
            profile_operation: Arc::new(Mutex::new(())),
            next_session_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
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
        // Through the shared compiled-world cache when the operator
        // configured one (design #5965 section 2, P1 in #6235): every
        // compile is an admin compile today, so entries are attributed
        // to the house author at author scope — unpublished source's
        // default (section 2.3) — and identical recompiles skip the
        // compiler entirely.
        let compiled = if compiler.cache().is_some() {
            crate::world_compile::compile_with_cache(
                spec,
                compiler,
                &granted,
                COMPILE_TIMEOUT,
                crate::world_compile::ADMIN_CACHE_AUTHOR,
                crate::world_cache::CacheScope::Author,
            )
            .await
        } else {
            crate::world_compile::compile(spec, compiler, &granted, COMPILE_TIMEOUT).await
        };
        let (wasm, diagnostics, provenance) = compiled.map_err(|error| {
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
                None,
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
    pub(super) fn try_reserve_module_upload(&self) -> Result<OwnedSemaphorePermit, InstallError> {
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
        self.install_wasm_with_signature(supplied_admin, wasm, seed, claimed_hash, None)
            .await
    }

    /// As [`Self::install_wasm`], with an optional signer signature for
    /// servers that configure a non-empty module trust list.
    #[cfg(feature = "sandbox")]
    pub async fn install_wasm_with_signature(
        &self,
        supplied_admin: Option<&str>,
        wasm: Vec<u8>,
        seed: u64,
        claimed_hash: &str,
        signature: Option<&ModuleSignature>,
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
            signature,
        )
        .await
    }

    #[cfg(feature = "sandbox")]
    pub(super) async fn install_wasm_reserved(
        &self,
        supplied_admin: Option<&str>,
        wasm: Vec<u8>,
        seed: u64,
        claimed_hash: &str,
        _permit: OwnedSemaphorePermit,
        compiled_from_source: bool,
        provenance: Option<crate::world::ModuleProvenance>,
        signature: Option<&ModuleSignature>,
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
        // The trust gate governs uploaded artifacts. A module the server
        // compiled itself from source (`compiled_from_source`) never crosses
        // the upload path, so it needs no publisher signature — requiring one
        // would disable source-only program updates entirely (Codex review
        // on PR #4088).
        let trust_decision = if compiled_from_source {
            crate::world::TrustDecision {
                basis: crate::world::TrustBasis::ServerCompiled,
                publisher: None,
            }
        } else {
            verify_module_signature(&self.trusted_module_keys, &wasm, signature)?;
            if self.trusted_module_keys.is_empty() {
                crate::world::TrustDecision {
                    basis: crate::world::TrustBasis::NoTrustList,
                    publisher: None,
                }
            } else {
                crate::world::TrustDecision {
                    basis: crate::world::TrustBasis::SignatureVerified,
                    publisher: signature.map(|sig| sig.public_key.clone()),
                }
            }
        };
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
            .install_program_with_trust(
                module_hash.clone(),
                seed,
                Some(provenance.unwrap_or(crate::world::ModuleProvenance::Artifact)),
                None,
                Some(trust_decision),
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

    /// Install an uploaded Wasm module on the authority of a joined
    /// builder's live capability (design #835 phase 3). The upload
    /// safeguards are the admin path's — size and hash bounds, the
    /// module trust gate, source-only refusal — with two differences:
    /// no admin token is involved (the world actor authorizes the
    /// capability at the effect boundary and derives the
    /// `BuilderInstalled { by }` audit actor from it), and builder
    /// installs are **Wasm-only**: a native artifact is refused even
    /// when this host enables native execution, so builder authority
    /// can never reach the native installer.
    #[cfg(feature = "sandbox")]
    pub async fn install_wasm_authorized(
        &self,
        token: JoinCapability,
        wasm: Vec<u8>,
        seed: u64,
        claimed_hash: &str,
        _permit: OwnedSemaphorePermit,
        signature: Option<&ModuleSignature>,
    ) -> Result<(String, u64), InstallError> {
        let _profile_operation = self.profile_operation.lock().await;
        if self.source_only {
            return Err(InstallError::RawModuleDisabled);
        }
        if wasm.is_empty() || wasm.len() > MAX_WORLD_MODULE_BYTES {
            return Err(InstallError::InvalidUpload);
        }
        let hash = Hash::new(&wasm);
        let module_hash = hash.to_string();
        if claimed_hash != module_hash {
            return Err(InstallError::InvalidUpload);
        }
        // The same publisher trust gate as an admin upload, and the
        // same trust decision recorded for it: a builder install the
        // gate admitted on a verified signature must journal that
        // decision (basis + publisher) in its install event, exactly
        // as the admin path's does (#1069).
        verify_module_signature(&self.trusted_module_keys, &wasm, signature)?;
        let trust_decision = if self.trusted_module_keys.is_empty() {
            crate::world::TrustDecision {
                basis: crate::world::TrustBasis::NoTrustList,
                publisher: None,
            }
        } else {
            crate::world::TrustDecision {
                basis: crate::world::TrustBasis::SignatureVerified,
                publisher: signature.map(|sig| sig.public_key.clone()),
            }
        };
        if crate::world_native::is_native_module(&wasm) {
            return Err(InstallError::InvalidModule);
        }
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
        let guest: Box<dyn crate::world::WorldProgram> = Box::new(program);

        // A durable world must be able to reload this module before
        // the journal may reference it.
        if let Some(modules) = self.module_dir.clone() {
            let bytes = wasm.clone();
            let saved = tokio::task::spawn_blocking(move || modules.save(&bytes))
                .await
                .map_err(|_| InstallError::Unavailable)?
                .map_err(|_| InstallError::Unavailable)?;
            if saved != module_hash {
                return Err(InstallError::Unavailable);
            }
        }

        let blobs = self.blobs.as_ref().ok_or(InstallError::Disabled)?.clone();
        let pin = blobs
            .add_bytes_with_opts(AddBytesOptions {
                data: wasm.into(),
                format: BlobFormat::Raw,
            })
            .temp_tag()
            .await
            .map_err(|_| InstallError::Unavailable)?;
        if pin.hash() != hash {
            return Err(InstallError::Unavailable);
        }

        // The effect boundary: the authorized command re-checks the
        // capability inside the actor, and its own answer classifies
        // the outcome — a denial the command itself returns (the
        // capability expired, or a revocation landed, after the early
        // check below) carries the stable `actor_denied` code, told
        // apart from an activation failure by the typed denial in the
        // command's error. The early check only avoids receiving an
        // upload the command would certainly refuse; it never widens
        // what the command allows (fail-closed).
        self.world
            .authorize(token.clone(), WorldScopes::BUILDER)
            .await
            .map_err(|_| InstallError::ActorDenied)?;
        let installed = self
            .world
            .install_program_authorized(
                token,
                module_hash.clone(),
                seed,
                Some(crate::world::ModuleProvenance::Artifact),
                Some(trust_decision),
                guest,
            )
            .await
            .map_err(|error| {
                if crate::world::is_capability_denied(&error) {
                    InstallError::ActorDenied
                } else {
                    InstallError::Unavailable
                }
            })?;
        if !matches!(
            installed.kind,
            crate::world::WorldEventKind::ProgramInstalled { .. }
        ) {
            return Err(InstallError::Unavailable);
        }
        *self.module_pin.lock().await = Some(pin);
        *self.module_hash.write().await = Some(module_hash.clone());
        Ok((module_hash, installed.sequence))
    }


    /// Read the currently pinned module artifact for peer download.
    /// The caller must already have authenticated as a world participant.
    ///
    /// Falls back to the module directory when there is no blob pin to
    /// serve from. A read-only replica clears its blob store so module
    /// uploads stay refused, but the module it restored was loaded out of
    /// its module directory, so spectators can still download it from
    /// there (PR #4708 Codex P2).
    pub async fn module_bytes(&self, expected_hash: &str) -> Result<Vec<u8>, InstallError> {
        if let Some(blobs) = self.blobs.as_ref() {
            let pin = self.module_pin.lock().await;
            match pin.as_ref() {
                Some(pin) if pin.hash().to_string() == expected_hash => {
                    let bytes = blobs
                        .blobs()
                        .get_bytes(pin.hash())
                        .await
                        .map_err(|_| InstallError::Unavailable)?;
                    if bytes.len() > MAX_WORLD_MODULE_BYTES || module_hash(&bytes) != expected_hash
                    {
                        return Err(InstallError::Unavailable);
                    }
                    return Ok(bytes.to_vec());
                }
                Some(_) => return Err(InstallError::InvalidUpload),
                None => {}
            }
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
            return Err(InstallError::Unavailable);
        }
        if self.blobs.is_none() {
            return Err(InstallError::Disabled);
        }
        Err(InstallError::Unavailable)
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
