//! Minimal, capability-free Wasm runner for world state transitions.
//!
//! The guest ABI follows the Roc/Zig adapter: `plaza_init(i64) -> i32`,
//! `plaza_update(i32, i32, i32, i32) -> i32` (model, participant, event),
//! `plaza_records(i32) -> i32`, `plaza_view(i32, i32, i32) -> i32`,
//! `plaza_out_len() -> i32`, plus `memory`, `plaza_alloc(i32) -> i32`, and
//! `plaza_free(i32, i32)`. Each world owns one persistent Wasmtime instance;
//! the model pointer stays inside its guest linear memory.
//! The guest receives no imports: no WASI, filesystem, sockets, clocks, or
//! random source. This byte ABI is deliberately separate from Roc's internal
//! ABI; a versioned Roc platform adapter must compile to this boundary.

use anyhow::{Context, Result, ensure};
use wasmtime::{
    Config, Engine, Instance, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc,
};

/// Resource bounds applied to each guest invocation.
#[derive(Clone, Copy, Debug)]
pub struct SandboxLimits {
    /// Maximum compiled Wasm module size, in bytes.
    pub module_bytes: usize,
    /// Maximum Wasm fuel for an invocation.
    pub fuel: u64,
    /// Maximum linear memory, in bytes.
    pub memory_bytes: usize,
    /// Maximum input or output payload, in bytes.
    pub message_bytes: usize,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            module_bytes: 16 * 1024 * 1024,
            fuel: 10_000_000,
            memory_bytes: 16 * 1024 * 1024,
            message_bytes: 1024 * 1024,
        }
    }
}

/// Validated, compiled, import-free Wasm module.
#[derive(Clone, Debug)]
pub struct Sandbox {
    engine: Engine,
    module: Module,
    limits: SandboxLimits,
}

impl Sandbox {
    /// Compile and validate a guest module against the no-import policy.
    pub fn compile(wasm: &[u8], limits: SandboxLimits) -> Result<Self> {
        ensure!(
            wasm.len() <= limits.module_bytes,
            "guest module exceeds {} bytes",
            limits.module_bytes
        );
        ensure!(limits.fuel > 0, "sandbox fuel must be nonzero");
        ensure!(
            limits.memory_bytes >= 64 * 1024,
            "sandbox memory limit must be at least one Wasm page"
        );
        ensure!(
            limits.message_bytes > 0,
            "sandbox message limit must be nonzero"
        );

        let mut config = Config::new();
        config.consume_fuel(true);
        config.max_wasm_stack(512 * 1024);
        config.wasm_multi_memory(false);
        config.wasm_memory64(false);
        let engine =
            Engine::new(&config).map_err(|e| anyhow::anyhow!("create Wasmtime engine: {e}"))?;
        let module =
            Module::new(&engine, wasm).map_err(|e| anyhow::anyhow!("compile Wasm module: {e}"))?;
        let imports: Vec<_> = module
            .imports()
            .map(|i| format!("{}::{}", i.module(), i.name()))
            .collect();
        ensure!(
            imports.is_empty(),
            "guest imports are not allowed: {}",
            imports.join(", ")
        );
        let mut exports = std::collections::HashSet::new();
        for export in module.exports() {
            ensure!(
                matches!(
                    export.name(),
                    "memory"
                        | "plaza_alloc"
                        | "plaza_init"
                        | "plaza_update"
                        | "plaza_free"
                        | "plaza_view"
                        | "plaza_records"
                        | "plaza_wants"
                        | "plaza_snapshot"
                        | "plaza_restore"
                        | "plaza_out_len"
                        | "plaza_error_ptr"
                        | "plaza_error_len"
                ),
                "guest export `{}` is not in the ABI allowlist",
                export.name()
            );
            exports.insert(export.name());
        }
        for required in [
            "memory",
            "plaza_alloc",
            "plaza_free",
            "plaza_init",
            "plaza_update",
            "plaza_view",
            "plaza_out_len",
        ] {
            ensure!(
                exports.contains(required),
                "guest module is missing required export `{required}`"
            );
        }
        // Checkpointing needs both halves of the pair.
        ensure!(
            exports.contains("plaza_snapshot") == exports.contains("plaza_restore"),
            "guest must export both `plaza_snapshot` and `plaza_restore`, or neither"
        );
        Ok(Self {
            engine,
            module,
            limits,
        })
    }

    /// Create an isolated, long-lived guest instance for one world.
    pub fn instantiate(&self, seed: u64) -> Result<WorldInstance> {
        let state = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .table_elements(10_000)
            .instances(1)
            .tables(1)
            .memories(1)
            .trap_on_grow_failure(true)
            .build();
        let mut store = Store::new(&self.engine, state);
        store.limiter(|limits| limits);
        store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("set guest fuel: {e}"))?;

        let instance = Instance::new(&mut store, &self.module, &[]).map_err(|e| {
            anyhow::anyhow!("instantiate guest (start functions are fuel-limited): {e}")
        })?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow::anyhow!("guest must export linear memory as `memory`"))?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "plaza_alloc")
            .map_err(|e| anyhow::anyhow!("guest must export `plaza_alloc(i32) -> i32`: {e}"))?;
        let free = instance
            .get_typed_func::<(i32, i32), ()>(&mut store, "plaza_free")
            .map_err(|e| anyhow::anyhow!("guest must export `plaza_free(i32, i32)`: {e}"))?;
        let init = instance
            .get_typed_func::<i64, i32>(&mut store, "plaza_init")
            .map_err(|e| anyhow::anyhow!("guest must export `plaza_init(i64) -> i32`: {e}"))?;
        let update = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "plaza_update")
            .map_err(|e| {
                anyhow::anyhow!("guest must export `plaza_update(i32, i32, i32, i32) -> i32`: {e}")
            })?;
        let view = instance
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, "plaza_view")
            .map_err(|e| {
                anyhow::anyhow!("guest must export `plaza_view(i32, i32, i32) -> i32`: {e}")
            })?;
        let out_len = instance
            .get_typed_func::<(), i32>(&mut store, "plaza_out_len")
            .map_err(|e| anyhow::anyhow!("guest must export `plaza_out_len() -> i32`: {e}"))?;
        // Optional: a pure projection of the model into structured records.
        let records = if instance.get_func(&mut store, "plaza_records").is_some() {
            Some(
                instance
                    .get_typed_func::<i32, i32>(&mut store, "plaza_records")
                    .map_err(|e| {
                        anyhow::anyhow!("guest `plaza_records` must be `(i32) -> i32`: {e}")
                    })?,
            )
        } else {
            None
        };
        // Optional: what the program wants from the server (see world_caps).
        let wants = if instance.get_func(&mut store, "plaza_wants").is_some() {
            Some(
                instance
                    .get_typed_func::<i32, i32>(&mut store, "plaza_wants")
                    .map_err(|e| {
                        anyhow::anyhow!("guest `plaza_wants` must be `(i32) -> i32`: {e}")
                    })?,
            )
        } else {
            None
        };
        // Optional pair: serialize the model, and rebuild it from that text.
        let (snapshot, restore) = if instance.get_func(&mut store, "plaza_snapshot").is_some() {
            let snapshot = instance
                .get_typed_func::<i32, i32>(&mut store, "plaza_snapshot")
                .map_err(|e| {
                    anyhow::anyhow!("guest `plaza_snapshot` must be `(i32) -> i32`: {e}")
                })?;
            let restore = instance
                .get_typed_func::<(i32, i32), i32>(&mut store, "plaza_restore")
                .map_err(|e| {
                    anyhow::anyhow!("guest `plaza_restore` must be `(i32, i32) -> i32`: {e}")
                })?;
            (Some(snapshot), Some(restore))
        } else {
            (None, None)
        };
        let error_ptr = if instance.get_func(&mut store, "plaza_error_ptr").is_some() {
            Some(
                instance
                    .get_typed_func::<(), i32>(&mut store, "plaza_error_ptr")
                    .map_err(|e| {
                        anyhow::anyhow!("guest `plaza_error_ptr` has invalid signature: {e}")
                    })?,
            )
        } else {
            None
        };
        let error_len = if instance.get_func(&mut store, "plaza_error_len").is_some() {
            Some(
                instance
                    .get_typed_func::<(), i32>(&mut store, "plaza_error_len")
                    .map_err(|e| {
                        anyhow::anyhow!("guest `plaza_error_len` has invalid signature: {e}")
                    })?,
            )
        } else {
            None
        };

        let model = init.call(&mut store, seed.cast_signed()).map_err(|e| {
            anyhow::anyhow!("guest `plaza_init` failed (trap or fuel exhausted): {e}")
        })?;
        Ok(WorldInstance {
            runner: self.clone(),
            seed,
            limits: self.limits,
            store,
            memory,
            alloc,
            free,
            update,
            view,
            records,
            wants,
            snapshot,
            restore,
            out_len,
            error_ptr,
            error_len,
            model,
            poisoned: false,
        })
    }
}

/// Persistent guest state for a single authoritative world.
pub struct WorldInstance {
    /// The compiled module, so a snapshot can be proven on a probe instance.
    runner: Sandbox,
    seed: u64,
    limits: SandboxLimits,
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    update: TypedFunc<(i32, i32, i32, i32), i32>,
    view: TypedFunc<(i32, i32, i32), i32>,
    records: Option<TypedFunc<i32, i32>>,
    wants: Option<TypedFunc<i32, i32>>,
    snapshot: Option<TypedFunc<i32, i32>>,
    restore: Option<TypedFunc<(i32, i32), i32>>,
    out_len: TypedFunc<(), i32>,
    error_ptr: Option<TypedFunc<(), i32>>,
    error_len: Option<TypedFunc<(), i32>>,
    model: i32,
    poisoned: bool,
}

impl std::fmt::Debug for WorldInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldInstance")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl WorldInstance {
    /// Apply one event from `participant`. The model pointer is committed
    /// only if guest update succeeds.
    pub fn update(&mut self, participant: u64, event: &[u8]) -> Result<()> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        self.check_message_size(event)?;
        // The world's participant limit keeps IDs far below i32::MAX; clamp
        // defensively so a hostile caller can never wrap the guest argument.
        let participant = i32::try_from(participant).unwrap_or(i32::MAX);
        let result = self.update_inner(participant, event);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn update_inner(&mut self, participant: i32, event: &[u8]) -> Result<()> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let GuestBuffer {
            ptr,
            len,
            allocation_len,
        } = self.copy_input(event)?;
        let result = self
            .update
            .call(&mut self.store, (self.model, participant, ptr, len));
        let model = match result {
            Ok(model) => model,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "guest `plaza_update` failed (trap or fuel exhausted): {e}{}",
                    self.guest_error_suffix()
                ));
            }
        };
        self.free_input(ptr, allocation_len)
            .map_err(|e| anyhow::anyhow!("free guest event buffer: {e}"))?;
        self.model = model;
        Ok(())
    }

    /// Render this world's current state for an opaque viewer description.
    pub fn view(&mut self, viewer: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        self.check_message_size(viewer)?;
        let result = self.view_inner(viewer);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn view_inner(&mut self, viewer: &[u8]) -> Result<Vec<u8>> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let GuestBuffer {
            ptr,
            len,
            allocation_len,
        } = self.copy_input(viewer)?;
        let rendered = self.view.call(&mut self.store, (self.model, ptr, len));
        let output_ptr = match rendered {
            Ok(ptr) => ptr,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "guest `plaza_view` failed (trap or fuel exhausted): {e}{}",
                    self.guest_error_suffix()
                ));
            }
        };
        self.free_input(ptr, allocation_len)
            .map_err(|e| anyhow::anyhow!("free guest viewer buffer: {e}"))?;
        self.read_output(output_ptr)
    }

    /// The guest's structured records, or `None` if it does not export them.
    pub fn records(&mut self) -> Result<Option<Vec<u8>>> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        let Some(records) = self.records.clone() else {
            return Ok(None);
        };
        let result = self.records_inner(&records);
        if result.is_err() {
            self.poisoned = true;
        }
        result.map(Some)
    }

    fn records_inner(&mut self, records: &TypedFunc<i32, i32>) -> Result<Vec<u8>> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let output_ptr = records.call(&mut self.store, self.model).map_err(|e| {
            anyhow::anyhow!(
                "guest `plaza_records` failed (trap or fuel exhausted): {e}{}",
                self.guest_error_suffix()
            )
        })?;
        self.read_output(output_ptr)
    }

    /// The guest's current `wants` document, or `None` if it exports none.
    pub fn wants(&mut self) -> Result<Option<Vec<u8>>> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        let Some(wants) = self.wants.clone() else {
            return Ok(None);
        };
        let result = self.wants_inner(&wants);
        if result.is_err() {
            self.poisoned = true;
        }
        result.map(Some)
    }

    fn wants_inner(&mut self, wants: &TypedFunc<i32, i32>) -> Result<Vec<u8>> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let output_ptr = wants.call(&mut self.store, self.model).map_err(|e| {
            anyhow::anyhow!(
                "guest `plaza_wants` failed (trap or fuel exhausted): {e}{}",
                self.guest_error_suffix()
            )
        })?;
        self.read_output(output_ptr)
    }

    /// The guest's serialized model, or `None` if it exports no
    /// snapshot/restore pair.
    pub fn snapshot(&mut self) -> Result<Option<Vec<u8>>> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        let Some(snapshot) = self.snapshot.clone() else {
            return Ok(None);
        };
        let result = self.snapshot_inner(&snapshot);
        if result.is_err() {
            self.poisoned = true;
        }
        result.map(Some)
    }

    fn snapshot_inner(&mut self, snapshot: &TypedFunc<i32, i32>) -> Result<Vec<u8>> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let output_ptr = snapshot.call(&mut self.store, self.model).map_err(|e| {
            anyhow::anyhow!(
                "guest `plaza_snapshot` failed (trap or fuel exhausted): {e}{}",
                self.guest_error_suffix()
            )
        })?;
        self.read_output(output_ptr)
    }

    /// Replace the model with one rebuilt from `snapshot`.
    ///
    /// Intended for a freshly instantiated guest; the model it replaces is
    /// simply abandoned inside the guest's memory.
    pub fn restore(&mut self, snapshot: &[u8]) -> Result<()> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        let Some(restore) = self.restore.clone() else {
            anyhow::bail!("guest exports no `plaza_restore`");
        };
        self.check_message_size(snapshot)?;
        let result = self.restore_inner(&restore, snapshot);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn restore_inner(
        &mut self,
        restore: &TypedFunc<(i32, i32), i32>,
        snapshot: &[u8],
    ) -> Result<()> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let GuestBuffer {
            ptr,
            len,
            allocation_len,
        } = self.copy_input(snapshot)?;
        let model = restore.call(&mut self.store, (ptr, len)).map_err(|e| {
            anyhow::anyhow!(
                "guest `plaza_restore` failed (trap or fuel exhausted): {e}{}",
                self.guest_error_suffix()
            )
        })?;
        self.free_input(ptr, allocation_len)
            .map_err(|e| anyhow::anyhow!("free guest snapshot buffer: {e}"))?;
        self.model = model;
        Ok(())
    }

    /// Copy the guest's current output buffer (`plaza_out_len` bytes at
    /// `output_ptr`) out of linear memory, bounds-checked.
    fn read_output(&mut self, output_ptr: i32) -> Result<Vec<u8>> {
        let output_len = self
            .out_len
            .call(&mut self.store, ())
            .map_err(|e| anyhow::anyhow!("guest `plaza_out_len` failed: {e}"))?;
        ensure!(output_len >= 0, "guest returned a negative output length");
        let output_len =
            usize::try_from(output_len).context("guest output length does not fit host usize")?;
        ensure!(
            output_len <= self.limits.message_bytes,
            "guest output exceeds {} bytes",
            self.limits.message_bytes
        );
        let output_ptr = output_ptr.cast_unsigned() as usize;
        let end = output_ptr
            .checked_add(output_len)
            .context("guest output range overflow")?;
        ensure!(
            end <= self.memory.data_size(&self.store),
            "guest returned output outside linear memory"
        );
        let mut output = vec![0; output_len];
        self.memory
            .read(&self.store, output_ptr, &mut output)
            .map_err(|e| anyhow::anyhow!("read guest output: {e}"))?;
        Ok(output)
    }

    fn check_message_size(&self, message: &[u8]) -> Result<()> {
        ensure!(
            message.len() <= self.limits.message_bytes,
            "guest message exceeds {} bytes",
            self.limits.message_bytes
        );
        Ok(())
    }

    fn copy_input(&mut self, input: &[u8]) -> Result<GuestBuffer> {
        let len = i32::try_from(input.len()).context("guest input does not fit wasm32 ABI")?;
        // Roc's Str adapter requires a valid pointer even for the empty string.
        // Allocate one byte but pass the logical length (zero) to the guest.
        let allocation_len = len.max(1);
        let ptr = self
            .alloc
            .call(&mut self.store, allocation_len)
            .map_err(|e| anyhow::anyhow!("guest allocation failed: {e}"))?;
        if !input.is_empty() {
            self.memory
                .write(&mut self.store, ptr.cast_unsigned() as usize, input)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "write guest input at {:#x} ({} bytes, memory {}): {e}",
                        ptr.cast_unsigned(),
                        input.len(),
                        self.memory.data_size(&self.store)
                    )
                })?;
        }
        Ok(GuestBuffer {
            ptr,
            len,
            allocation_len,
        })
    }

    fn free_input(&mut self, ptr: i32, len: i32) -> Result<()> {
        self.free
            .call(&mut self.store, (ptr, len))
            .map_err(|e| anyhow::anyhow!("free guest input buffer: {e}"))
    }

    /// Read optional diagnostic exports after a guest trap. This invokes only
    /// no-argument, guest-local getters; no host capability is introduced.
    fn guest_error_suffix(&mut self) -> String {
        let (Some(ptr_fn), Some(len_fn)) = (&self.error_ptr, &self.error_len) else {
            return String::new();
        };
        if self.store.set_fuel(10_000).is_err() {
            return String::new();
        }
        let (Ok(ptr), Ok(len)) = (
            ptr_fn.call(&mut self.store, ()),
            len_fn.call(&mut self.store, ()),
        ) else {
            return String::new();
        };
        let Ok(len) = usize::try_from(len) else {
            return String::new();
        };
        if len == 0 || len > 4096 {
            return String::new();
        }
        let ptr = ptr.cast_unsigned() as usize;
        let Some(end) = ptr.checked_add(len) else {
            return String::new();
        };
        if end > self.memory.data_size(&self.store) {
            return String::new();
        }
        let mut message = vec![0; len];
        if self.memory.read(&self.store, ptr, &mut message).is_err() {
            return String::new();
        }
        format!("; guest diagnostic: {}", String::from_utf8_lossy(&message))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
impl WorldInstance {
    fn snapshot_via_trait(&mut self) -> Option<String> {
        crate::world::WorldProgram::snapshot(self).unwrap()
    }
}

impl crate::world::WorldProgram for WorldInstance {
    fn update(&mut self, event: &crate::world::WorldEvent) -> Result<()> {
        match &event.kind {
            crate::world::WorldEventKind::Input(bytes) => {
                std::str::from_utf8(bytes).context("Roc world input must be valid UTF-8")?;
                WorldInstance::update(self, event.participant_id, bytes)
            }
            // Host events (capability results, ticks, presence) are JSON
            // text delivered as participant 0; the program updates on them
            // like any other event.
            crate::world::WorldEventKind::System { event } => {
                WorldInstance::update(self, 0, event.as_bytes())
            }
            crate::world::WorldEventKind::Chat(_)
            | crate::world::WorldEventKind::ProgramInstalled { .. } => Ok(()),
        }
    }

    fn view(&mut self, viewer: &crate::world::Participant) -> Result<Option<String>> {
        let viewer = serde_json::to_vec(viewer).context("serialize Roc viewer context")?;
        let output = WorldInstance::view(self, &viewer)?;
        String::from_utf8(output)
            .map(Some)
            .context("Roc world view must return valid UTF-8")
    }

    fn records(&mut self) -> Result<Option<String>> {
        WorldInstance::records(self)?
            .map(|bytes| String::from_utf8(bytes).context("Roc world records must be valid UTF-8"))
            .transpose()
    }

    fn wants(&mut self) -> Result<Option<String>> {
        WorldInstance::wants(self)?
            .map(|bytes| String::from_utf8(bytes).context("Roc world wants must be valid UTF-8"))
            .transpose()
    }

    /// Serialize the model, but only hand it out after proving it round-trips
    /// on a probe instance: restoring it must reproduce the same snapshot and
    /// the same records. A journal trimmed on an unfaithful snapshot would
    /// silently change the world's state on restart.
    fn snapshot(&mut self) -> Result<Option<String>> {
        let Some(bytes) = WorldInstance::snapshot(self)? else {
            return Ok(None);
        };
        let live_records = WorldInstance::records(self)?;
        let mut probe = self.runner.instantiate(self.seed)?;
        probe.restore(&bytes).context("verify snapshot: restore")?;
        let again = probe
            .snapshot()
            .context("verify snapshot: re-snapshot")?
            .context("verify snapshot: probe lost its snapshot export")?;
        ensure!(again == bytes, "snapshot is not stable under restore");
        ensure!(
            probe.records().context("verify snapshot: records")? == live_records,
            "restored guest publishes different records"
        );
        String::from_utf8(bytes)
            .map(Some)
            .context("Roc world snapshot must be valid UTF-8")
    }
}

struct GuestBuffer {
    ptr: i32,
    len: i32,
    allocation_len: i32,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const ECHO: &str = r#"
        (module
          (memory (export "memory") 1 1)
          (data (i32.const 32) "hello")
          (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
          (func (export "plaza_free") (param i32 i32))
          (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
          (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
            local.get 0)
          (func (export "plaza_view") (param i32 i32 i32) (result i32)
            (i32.const 32))
          (func (export "plaza_out_len") (result i32) (i32.const 5)))
    "#;

    fn compile(wat: &str, limits: SandboxLimits) -> Result<Sandbox> {
        let wasm = wat::parse_str(wat)?;
        Sandbox::compile(&wasm, limits)
    }

    #[test]
    fn import_free_guest_runs_and_returns_bounded_bytes() {
        let runner = compile(ECHO, SandboxLimits::default()).unwrap();
        let mut world = runner.instantiate(123).unwrap();
        assert_eq!(world.view(b"viewer").unwrap(), b"hello");
        world.update(1, b"event").unwrap();
        assert_eq!(world.view(b"viewer").unwrap(), b"hello");
    }

    #[test]
    fn checked_in_roc_counter_compiles_and_initializes_in_the_import_free_sandbox() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut world = runner.instantiate(7).unwrap();
        assert_eq!(world.view(b"").unwrap(), b"count=0");
        world.update(1, b"inc").unwrap();
        assert_eq!(world.view(b"").unwrap(), b"count=1");
        world.update(1, b"inc").unwrap();
        assert_eq!(world.view(b"").unwrap(), b"count=2");
    }

    #[tokio::test]
    async fn checked_in_roc_counter_runs_as_the_authoritative_world_program() {
        use crate::world::{WorldCore, WorldHandle, WorldLimits, WorldScopes};

        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let guest = runner.instantiate(7).unwrap();
        let actor = WorldHandle::spawn_with_program(
            WorldCore::new("counter", WorldLimits::default()).unwrap(),
            Box::new(guest),
        );
        let (_, capability) = actor.issue("Ada", WorldScopes::GUEST).await.unwrap();

        assert_eq!(
            actor.view(capability.clone()).await.unwrap().as_deref(),
            Some("count=0")
        );
        actor
            .input(capability.clone(), b"inc".to_vec())
            .await
            .unwrap();
        assert_eq!(
            actor.view(capability.clone()).await.unwrap().as_deref(),
            Some("count=1")
        );
        actor
            .chat(capability.clone(), "chat does not mutate the game")
            .await
            .unwrap();
        assert_eq!(
            actor.view(capability).await.unwrap().as_deref(),
            Some("count=1")
        );
    }

    #[test]
    fn checked_in_roc_counter_publishes_records() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut world = runner.instantiate(7).unwrap();
        assert_eq!(world.records().unwrap().unwrap(), br#"{"count":0}"#);
        world.update(1, b"inc").unwrap();
        world.update(1, b"inc").unwrap();
        assert_eq!(world.records().unwrap().unwrap(), br#"{"count":2}"#);
        // A view between records calls must not disturb either output.
        assert_eq!(world.view(b"").unwrap(), b"count=2");
        assert_eq!(world.records().unwrap().unwrap(), br#"{"count":2}"#);
    }

    #[test]
    fn checked_in_tic_tac_toe_plays_a_full_game() {
        let wasm = include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut world = runner.instantiate(7).unwrap();
        let records = |world: &mut WorldInstance| {
            String::from_utf8(world.records().unwrap().unwrap()).unwrap()
        };
        assert_eq!(
            records(&mut world),
            r#"{"board":"---------","plays":0,"winner":""}"#
        );

        // X plays 0, O plays 3, X 1, O 4, X 2 — top row for X.
        for (participant, cell) in [(1u64, "0"), (2, "3"), (1, "1"), (2, "4"), (1, "2")] {
            world.update(participant, cell.as_bytes()).unwrap();
        }
        assert_eq!(
            records(&mut world),
            r#"{"board":"XXXOO----","plays":5,"winner":"X"}"#
        );
        assert!(
            String::from_utf8(world.view(b"").unwrap())
                .unwrap()
                .contains("winner=X")
        );

        // Once the game is won, further moves change nothing.
        world.update(2, b"5").unwrap();
        assert_eq!(
            records(&mut world),
            r#"{"board":"XXXOO----","plays":5,"winner":"X"}"#
        );

        // Occupied cells, off-board cells, and non-digits are all ignored.
        let mut world = runner.instantiate(7).unwrap();
        world.update(1, b"0").unwrap();
        world.update(2, b"0").unwrap();
        world.update(2, b"9").unwrap();
        world.update(2, b"banana").unwrap();
        assert_eq!(
            records(&mut world),
            r#"{"board":"X--------","plays":1,"winner":""}"#
        );
    }

    #[test]
    fn checked_in_apps_snapshot_and_restore_faithfully() {
        // Counter: the model is its count.
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        for _ in 0..3 {
            live.update(1, b"inc").unwrap();
        }
        assert_eq!(live.snapshot().unwrap().unwrap(), b"3");
        let mut restored = runner.instantiate(7).unwrap();
        restored.restore(b"3").unwrap();
        assert_eq!(restored.view(b"").unwrap(), b"count=3");
        restored.update(1, b"inc").unwrap();
        assert_eq!(restored.view(b"").unwrap(), b"count=4");
        // The trait-level snapshot is the verified one the actor journals.
        assert_eq!(live.snapshot_via_trait(), Some("3".to_owned()));
        // Garbage restores to a defined state instead of trapping.
        let mut garbage = runner.instantiate(7).unwrap();
        garbage.restore(b"not a number").unwrap();
        assert_eq!(garbage.view(b"").unwrap(), b"count=0");

        // Tic-tac-toe: the board alone carries the ply count and the winner.
        let wasm = include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        for (participant, cell) in [(1u64, "0"), (2, "3"), (1, "1"), (2, "4"), (1, "2")] {
            live.update(participant, cell.as_bytes()).unwrap();
        }
        assert_eq!(live.snapshot().unwrap().unwrap(), b"XXXOO----");
        let mut restored = runner.instantiate(7).unwrap();
        restored.restore(b"XXXOO----").unwrap();
        assert_eq!(restored.records().unwrap(), live.records().unwrap());
        restored.update(2, b"5").unwrap();
        assert_eq!(
            restored.records().unwrap().unwrap(),
            br#"{"board":"XXXOO----","plays":5,"winner":"X"}"#,
            "the restored game is still won"
        );
        assert_eq!(live.snapshot_via_trait(), Some("XXXOO----".to_owned()));
        let mut bad = runner.instantiate(7).unwrap();
        bad.restore(b"short").unwrap();
        assert_eq!(
            bad.records().unwrap().unwrap(),
            br#"{"board":"---------","plays":0,"winner":""}"#
        );
    }

    #[test]
    fn the_lounge_uses_the_capability_protocol() {
        use crate::world::{WorldEvent, WorldEventKind};

        let wasm = include_bytes!("../examples/lounge/lounge.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut lounge = runner.instantiate(7).unwrap();
        fn wants_of(lounge: &mut WorldInstance) -> String {
            crate::world::WorldProgram::wants(lounge)
                .unwrap()
                .context("the lounge exports wants")
                .unwrap()
        }
        // At rest: two subscriptions, nothing pending.
        let document = wants_of(&mut lounge);
        assert!(document.contains("\"players\""), "{document}");
        assert!(document.contains("time.tick:5000"), "{document}");
        assert!(document.contains("\"requests\":[]"), "{document}");

        // An arrival (host event, participant 0) becomes a greeting request.
        let joined = serde_json::json!({
            "cap": "players",
            "event": "joined",
            "participant": {"id": 1, "name": "ann <script>"},
        });
        crate::world::WorldProgram::update(
            &mut lounge,
            &WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: joined.to_string(),
                },
            },
        )
        .unwrap();
        let document = wants_of(&mut lounge);
        assert!(document.contains("chat.say"), "{document}");
        assert!(
            document.contains("Welcome, ann script!") && !document.contains("<"),
            "the name is sanitized into the JSON: {document}"
        );

        // The world has not granted chat.say: the refusal is data the
        // program sees, and the request is retired.
        let denied = serde_json::json!({
            "cap": "result",
            "id": "r1",
            "ok": false,
            "error": "cap_denied",
        });
        crate::world::WorldProgram::update(
            &mut lounge,
            &WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: denied.to_string(),
                },
            },
        )
        .unwrap();
        assert!(wants_of(&mut lounge).contains("\"requests\":[]"));
        assert_eq!(
            crate::world::WorldProgram::records(&mut lounge)
                .unwrap()
                .unwrap(),
            r#"{"joins":1,"denied":1,"last_now":0}"#
        );

        // A granted request succeeds and retires itself.
        crate::world::WorldProgram::update(
            &mut lounge,
            &WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: serde_json::json!({
                        "cap": "players",
                        "event": "joined",
                        "participant": {"id": 2, "name": "bob"},
                    })
                    .to_string(),
                },
            },
        )
        .unwrap();
        let granted = serde_json::json!({
            "cap": "result",
            "id": "r2",
            "ok": true,
            "value": {"sequence": 9},
        });
        crate::world::WorldProgram::update(
            &mut lounge,
            &WorldEvent {
                sequence: 0,
                participant_id: 0,
                kind: WorldEventKind::System {
                    event: granted.to_string(),
                },
            },
        )
        .unwrap();
        assert_eq!(
            crate::world::WorldProgram::records(&mut lounge)
                .unwrap()
                .unwrap(),
            r#"{"joins":2,"denied":1,"last_now":0}"#
        );

        // The model survives a snapshot/restore round trip.
        let snapshot = crate::world::WorldProgram::snapshot(&mut lounge)
            .unwrap()
            .unwrap();
        let mut restored = runner.instantiate(7).unwrap();
        restored.restore(snapshot.as_bytes()).unwrap();
        assert_eq!(
            crate::world::WorldProgram::records(&mut restored)
                .unwrap()
                .unwrap(),
            r#"{"joins":2,"denied":1,"last_now":0}"#
        );
    }

    #[test]
    fn modules_without_snapshot_exports_do_not_snapshot() {
        let mut world = compile(ECHO, SandboxLimits::default())
            .unwrap()
            .instantiate(0)
            .unwrap();
        assert_eq!(world.snapshot().unwrap(), None);
        assert_eq!(world.snapshot_via_trait(), None);
        assert!(world.restore(b"x").is_err());
    }

    #[test]
    fn a_snapshot_export_without_restore_is_refused() {
        let half = format!(
            "{} (func (export \"plaza_snapshot\") (param i32) (result i32) (i32.const 0)))",
            ECHO.trim_end().strip_suffix(')').unwrap()
        );
        let error = compile(&half, SandboxLimits::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("plaza_restore"), "{error}");
    }

    #[test]
    fn records_export_is_optional() {
        let mut world = compile(ECHO, SandboxLimits::default())
            .unwrap()
            .instantiate(0)
            .unwrap();
        assert_eq!(world.records().unwrap(), None);
    }

    #[test]
    fn rejects_modules_with_host_imports() {
        let wasm = wat::parse_str(
            r#"(module (import "wasi_snapshot_preview1" "proc_exit" (func (param i32))))"#,
        )
        .unwrap();
        let error = Sandbox::compile(&wasm, SandboxLimits::default()).unwrap_err();
        assert!(error.to_string().contains("imports are not allowed"));
    }

    #[test]
    fn rejects_oversized_input_before_guest_execution() {
        let runner = compile(
            ECHO,
            SandboxLimits {
                message_bytes: 4,
                ..SandboxLimits::default()
            },
        )
        .unwrap();
        assert!(
            runner
                .instantiate(0)
                .unwrap()
                .view(b"12345")
                .unwrap_err()
                .to_string()
                .contains("exceeds")
        );
    }

    #[test]
    fn rejects_oversized_module_before_compilation() {
        let wasm = wat::parse_str(ECHO).unwrap();
        let error = Sandbox::compile(
            &wasm,
            SandboxLimits {
                module_bytes: wasm.len() - 1,
                ..SandboxLimits::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("module exceeds"));
    }

    #[test]
    fn rejects_guest_output_outside_memory() {
        let bad = ECHO.replace("(i32.const 32))", "(i32.const 131070))");
        let runner = compile(&bad, SandboxLimits::default()).unwrap();
        assert!(
            runner
                .instantiate(0)
                .unwrap()
                .view(b"")
                .unwrap_err()
                .to_string()
                .contains("outside linear memory")
        );
    }

    #[test]
    fn fuel_exhaustion_traps_instead_of_hanging() {
        let loop_module = r#"
            (module
              (memory (export "memory") 1 1)
              (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
              (func (export "plaza_free") (param i32 i32))
              (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
              (func (export "plaza_update") (param i32 i32 i32 i32) (result i32) (i32.const 0))
              (func (export "plaza_view") (param i32 i32 i32) (result i32) (i32.const 0)
                (loop $again (br $again)))
              (func (export "plaza_out_len") (result i32) (i32.const 0)))
        "#;
        let runner = compile(
            loop_module,
            SandboxLimits {
                fuel: 100,
                ..SandboxLimits::default()
            },
        )
        .unwrap();
        let mut world = runner.instantiate(0).unwrap();
        assert!(
            world
                .view(b"")
                .unwrap_err()
                .to_string()
                .contains("fuel exhausted")
        );
        assert!(
            world
                .view(b"")
                .unwrap_err()
                .to_string()
                .contains("poisoned")
        );
    }

    #[test]
    fn guest_memory_growth_is_bounded_and_poisons_world() {
        let memory_module = r#"
            (module
              (memory (export "memory") 1)
              (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
              (func (export "plaza_free") (param i32 i32))
              (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
              (func (export "plaza_update") (param i32 i32 i32 i32) (result i32)
                i32.const 1 memory.grow drop
                local.get 0)
              (func (export "plaza_view") (param i32 i32 i32) (result i32) (i32.const 0))
              (func (export "plaza_out_len") (result i32) (i32.const 0)))
        "#;
        let mut world = compile(
            memory_module,
            SandboxLimits {
                memory_bytes: 64 * 1024,
                ..SandboxLimits::default()
            },
        )
        .unwrap()
        .instantiate(0)
        .unwrap();
        let error = world.update(1, b"event").unwrap_err().to_string();
        assert!(error.contains("memory.grow") || error.contains("failed"));
        assert!(
            world
                .view(b"")
                .unwrap_err()
                .to_string()
                .contains("poisoned")
        );
    }
}
