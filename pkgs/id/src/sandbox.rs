//! Minimal, capability-free Wasm runner for world state transitions.
//!
//! The guest ABI follows the Roc/Zig adapter: `plaza_init(i64) -> i32`,
//! `plaza_update(i32, i32, i32, i32) -> i32` (model, participant, event),
//! `plaza_records(i32) -> i32`, `plaza_view(i32) -> i32`,
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

/// One kind of guest invocation, as counted by [`CallMetrics`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestCall {
    /// Module start function plus `plaza_init`, run once at instantiation.
    Init,
    /// `plaza_update`, including the input allocation and free around it.
    Update,
    /// `plaza_view`; also the fallback of [`WorldInstance::layout_at`] when
    /// the guest exports no `plaza_layout`.
    View,
    /// `plaza_layout`.
    Layout,
    /// `plaza_records`.
    Records,
    /// `plaza_wants`.
    Wants,
    /// `plaza_snapshot`.
    Snapshot,
    /// `plaza_restore`, including the snapshot allocation and free around it.
    Restore,
}

impl GuestCall {
    /// Every call kind, in the order metrics are reported.
    pub const ALL: [Self; 8] = [
        Self::Init,
        Self::Update,
        Self::View,
        Self::Layout,
        Self::Records,
        Self::Wants,
        Self::Snapshot,
        Self::Restore,
    ];

    /// Stable lowercase name used in reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Update => "update",
            Self::View => "view",
            Self::Layout => "layout",
            Self::Records => "records",
            Self::Wants => "wants",
            Self::Snapshot => "snapshot",
            Self::Restore => "restore",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Init => 0,
            Self::Update => 1,
            Self::View => 2,
            Self::Layout => 3,
            Self::Records => 4,
            Self::Wants => 5,
            Self::Snapshot => 6,
            Self::Restore => 7,
        }
    }
}

/// Fuel spent by one kind of guest call over a world instance's lifetime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallStats {
    /// Calls made, including ones that trapped.
    pub calls: u64,
    /// Fuel spent across all calls.
    pub fuel_total: u64,
    /// Most fuel any single call spent.
    pub fuel_max: u64,
    /// Fuel the most recent call spent.
    pub fuel_last: u64,
}

/// Per-call-kind fuel statistics for one [`WorldInstance`].
///
/// A fixed array indexed by [`GuestCall`]: recording a call allocates and
/// locks nothing. In-memory only; never journaled, sent, or snapshotted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallMetrics {
    stats: [CallStats; GuestCall::ALL.len()],
}

impl CallMetrics {
    /// Statistics for `call`; all zero if it was never made.
    pub const fn get(&self, call: GuestCall) -> CallStats {
        self.stats[call.index()]
    }

    /// Every call kind with its statistics, in [`GuestCall::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (GuestCall, CallStats)> + '_ {
        GuestCall::ALL
            .into_iter()
            .map(|call| (call, self.get(call)))
    }

    fn record(&mut self, call: GuestCall, fuel: u64) {
        let stats = &mut self.stats[call.index()];
        stats.calls = stats.calls.saturating_add(1);
        stats.fuel_total = stats.fuel_total.saturating_add(fuel);
        stats.fuel_max = stats.fuel_max.max(fuel);
        stats.fuel_last = fuel;
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
                        | "plaza_layout"
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
            .get_typed_func::<i32, i32>(&mut store, "plaza_view")
            .map_err(|e| anyhow::anyhow!("guest must export `plaza_view(i32) -> i32`: {e}"))?;
        // Optional: lay the presentation out for a connection's viewport.
        let layout = if instance.get_func(&mut store, "plaza_layout").is_some() {
            Some(
                instance
                    .get_typed_func::<(i32, i64, i64), i32>(&mut store, "plaza_layout")
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "guest `plaza_layout` must be `(i32, i64, i64) -> i32`: {e}"
                        )
                    })?,
            )
        } else {
            None
        };
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
        // One budget covers the start function and `plaza_init`: both ran
        // on the fuel set above.
        let mut metrics = CallMetrics::default();
        metrics.record(GuestCall::Init, fuel_spent(&store, self.limits.fuel));
        Ok(WorldInstance {
            runner: self.clone(),
            seed,
            limits: self.limits,
            metrics,
            trap_fuel: None,
            store,
            memory,
            alloc,
            free,
            update,
            view,
            layout,
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
    metrics: CallMetrics,
    /// Fuel a trapped call had spent, captured before the diagnostic getters
    /// refill the store; taken by [`Self::settle`].
    trap_fuel: Option<u64>,
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    update: TypedFunc<(i32, i32, i32, i32), i32>,
    view: TypedFunc<i32, i32>,
    layout: Option<TypedFunc<(i32, i64, i64), i32>>,
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

/// Fuel the store has spent since it was last set to `limit`. A call that
/// exhausted its fuel leaves the store at zero, which reads as the full limit.
fn fuel_spent(store: &Store<StoreLimits>, limit: u64) -> u64 {
    limit.saturating_sub(store.get_fuel().unwrap_or(limit))
}

impl WorldInstance {
    /// Fuel statistics per guest call kind since this instance was created.
    /// In-memory only: never journaled, sent on the wire, or snapshotted.
    pub const fn metrics(&self) -> &CallMetrics {
        &self.metrics
    }

    /// Fuel the most recent `call` spent; zero if it has not run.
    pub const fn last_fuel(&self, call: GuestCall) -> u64 {
        self.metrics.get(call).fuel_last
    }

    /// Record the fuel the call that just finished spent. Runs after every
    /// call that reached the guest, whether it succeeded or trapped.
    fn settle(&mut self, call: GuestCall) {
        let spent = self
            .trap_fuel
            .take()
            .unwrap_or_else(|| fuel_spent(&self.store, self.limits.fuel));
        self.metrics.record(call, spent);
    }

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
        self.settle(GuestCall::Update);
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

    /// Render this world's current state.
    pub fn view(&mut self) -> Result<Vec<u8>> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        let result = self.view_inner();
        self.settle(GuestCall::View);
        // A trapped view does not poison the instance: rendering never changes
        // the model, so the next event retries against intact state (F11.17).
        result
    }

    fn view_inner(&mut self) -> Result<Vec<u8>> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let rendered = self.view.call(&mut self.store, self.model);
        let output_ptr = match rendered {
            Ok(ptr) => ptr,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "guest `plaza_view` failed (trap or fuel exhausted): {e}{}",
                    self.guest_error_suffix()
                ));
            }
        };
        self.read_output(output_ptr)
    }

    /// Render this world at a viewport. Uses the guest's `plaza_layout` when it
    /// exports one, and otherwise the fixed [`Self::view`]. Rendering never
    /// changes the model or the journal.
    pub fn layout_at(&mut self, width: u16, height: u16) -> Result<Vec<u8>> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        let Some(layout) = self.layout.clone() else {
            return self.view();
        };
        let result = self.layout_inner(&layout, width, height);
        self.settle(GuestCall::Layout);
        // As with `view`, a trapped layout does not poison the instance.
        result
    }

    fn layout_inner(
        &mut self,
        layout: &TypedFunc<(i32, i64, i64), i32>,
        width: u16,
        height: u16,
    ) -> Result<Vec<u8>> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let model = self.model;
        let rendered = layout.call(
            &mut self.store,
            (model, i64::from(width), i64::from(height)),
        );
        let output_ptr = match rendered {
            Ok(ptr) => ptr,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "guest `plaza_layout` failed (trap or fuel exhausted): {e}{}",
                    self.guest_error_suffix()
                ));
            }
        };
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
        self.settle(GuestCall::Records);
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
        self.settle(GuestCall::Wants);
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
        self.settle(GuestCall::Snapshot);
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
        self.settle(GuestCall::Restore);
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
        // The trap left its fuel in the store; read it before the refill below.
        self.trap_fuel = Some(fuel_spent(&self.store, self.limits.fuel));
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
                Self::update(self, event.participant_id, bytes)
            }
            // Host events (capability results, ticks, presence) are JSON
            // text delivered as participant 0; the program updates on them
            // like any other event.
            crate::world::WorldEventKind::System { event } => {
                Self::update(self, 0, event.as_bytes())
            }
            crate::world::WorldEventKind::Chat(_)
            | crate::world::WorldEventKind::ProgramInstalled { .. } => Ok(()),
        }
    }

    fn view(&mut self, _viewer: &crate::world::Participant) -> Result<Option<String>> {
        let output = Self::view(self)?;
        String::from_utf8(output)
            .map(Some)
            .context("Roc world view must return valid UTF-8")
    }

    fn layout(
        &mut self,
        _viewer: &crate::world::Participant,
        width: u16,
        height: u16,
    ) -> Result<Option<String>> {
        let output = Self::layout_at(self, width, height)?;
        String::from_utf8(output)
            .map(Some)
            .context("Roc world layout must return valid UTF-8")
    }

    fn records(&mut self) -> Result<Option<String>> {
        Self::records(self)?
            .map(|bytes| String::from_utf8(bytes).context("Roc world records must be valid UTF-8"))
            .transpose()
    }

    fn wants(&mut self) -> Result<Option<String>> {
        Self::wants(self)?
            .map(|bytes| String::from_utf8(bytes).context("Roc world wants must be valid UTF-8"))
            .transpose()
    }

    /// Serialize the model, but only hand it out after proving it round-trips
    /// on a probe instance: restoring it must reproduce the same snapshot and
    /// the same records. A journal trimmed on an unfaithful snapshot would
    /// silently change the world's state on restart.
    fn snapshot(&mut self) -> Result<Option<String>> {
        let Some(bytes) = Self::snapshot(self)? else {
            return Ok(None);
        };
        let live_records = Self::records(self)?;
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
          (func (export "plaza_view") (param i32) (result i32)
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
        assert_eq!(world.view().unwrap(), b"hello");
        world.update(1, b"event").unwrap();
        assert_eq!(world.view().unwrap(), b"hello");
    }

    #[test]
    fn checked_in_roc_counter_compiles_and_initializes_in_the_import_free_sandbox() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut world = runner.instantiate(7).unwrap();
        assert!(
            String::from_utf8(world.view().unwrap())
                .unwrap()
                .contains("count=0")
        );
        world.update(1, b"inc").unwrap();
        assert!(
            String::from_utf8(world.view().unwrap())
                .unwrap()
                .contains("count=1")
        );
        world.update(1, b"inc").unwrap();
        assert!(
            String::from_utf8(world.view().unwrap())
                .unwrap()
                .contains("count=2")
        );
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

        assert!(
            actor
                .view(capability.clone())
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=0"))
        );
        actor
            .input(capability.clone(), b"inc".to_vec())
            .await
            .unwrap();
        assert!(
            actor
                .view(capability.clone())
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=1"))
        );
        actor
            .chat(capability.clone(), "chat does not mutate the game")
            .await
            .unwrap();
        assert!(
            actor
                .view(capability)
                .await
                .unwrap()
                .as_deref()
                .is_some_and(|view| view.contains("count=1"))
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
        assert!(
            String::from_utf8(world.view().unwrap())
                .unwrap()
                .contains("count=2")
        );
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
            String::from_utf8(world.view().unwrap())
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
        assert!(
            String::from_utf8(restored.view().unwrap())
                .unwrap()
                .contains("count=3")
        );
        restored.update(1, b"inc").unwrap();
        assert!(
            String::from_utf8(restored.view().unwrap())
                .unwrap()
                .contains("count=4")
        );
        // The trait-level snapshot is the verified one the actor journals.
        assert_eq!(live.snapshot_via_trait(), Some("3".to_owned()));
        // Garbage restores to a defined state instead of trapping.
        let mut garbage = runner.instantiate(7).unwrap();
        garbage.restore(b"not a number").unwrap();
        assert!(
            String::from_utf8(garbage.view().unwrap())
                .unwrap()
                .contains("count=0")
        );

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
    fn checked_in_roc_screen_renders_structured_surface() {
        let wasm = include_bytes!("../examples/roc-screen/roc-screen.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        live.update(1, b"+").unwrap();
        live.update(1, b"+").unwrap();
        let view_bytes = live.view().unwrap();
        let view_str = String::from_utf8(view_bytes).unwrap();
        let surface = crate::world_presentation::parse_surface(&view_str).unwrap();
        let Some(crate::world_presentation::WorldSurface::Screen(screen)) = surface else {
            panic!("expected structured Screen surface, got {view_str}");
        };
        assert_eq!((screen.width, screen.height), (20, 3));
        let rows: Vec<String> = screen
            .cells
            .chunks(usize::from(screen.width))
            .map(|row| row.iter().map(|cell| cell.symbol.as_str()).collect())
            .collect();
        assert!(rows[0].starts_with("+-counter"));
        assert!(rows[1].starts_with("| count=2"));
        assert_eq!(rows[2], format!("+{}+", "-".repeat(18)));
    }

    #[test]
    fn checked_in_grid_example_uses_bounded_get_set() {
        let wasm = include_bytes!("../examples/roc-grid/roc-grid.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        let render = |world: &mut WorldInstance| -> String {
            let view = String::from_utf8(world.view().unwrap()).unwrap();
            let Some(crate::world_presentation::WorldSurface::Screen(screen)) =
                crate::world_presentation::parse_surface(&view).unwrap()
            else {
                panic!("the grid example must return a structured screen: {view}");
            };
            screen
                .cells
                .chunks(usize::from(screen.width))
                .map(|row| {
                    row.iter()
                        .map(|cell| cell.symbol.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<String>>()
                .join("\n")
        };
        let text = render(&mut live);
        assert!(text.contains("cells=12 at11=46 out=63"), "{text}");
        live.update(1, b"set").unwrap();
        let text = render(&mut live);
        assert!(text.contains("cells=11 at11=65 out=63"), "{text}");
        // The snapshot is the grid's bytes, and restore rebuilds it.
        let snapshot = live.snapshot().unwrap().unwrap();
        let mut restored = runner.instantiate(7).unwrap();
        restored.restore(&snapshot).unwrap();
        let text = render(&mut restored);
        assert!(text.contains("cells=11 at11=65 out=63"), "{text}");
    }

    #[test]
    fn checked_in_chat_example_sends_and_reads_history() {
        use crate::world::{WorldEvent, WorldEventKind};

        let wasm = include_bytes!("../examples/roc-chat/roc-chat.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut chat = runner.instantiate(7).unwrap();
        fn wants_of(chat: &mut WorldInstance) -> String {
            crate::world::WorldProgram::wants(chat).unwrap().unwrap()
        }
        fn send(chat: &mut WorldInstance, ev: &[u8]) {
            crate::world::WorldProgram::update(
                chat,
                &WorldEvent {
                    sequence: 0,
                    participant_id: 1,
                    kind: WorldEventKind::Input(ev.to_vec()),
                },
            )
            .unwrap();
        }
        fn host(chat: &mut WorldInstance, event: serde_json::Value) {
            crate::world::WorldProgram::update(
                chat,
                &WorldEvent {
                    sequence: 0,
                    participant_id: 0,
                    kind: WorldEventKind::System {
                        event: event.to_string(),
                    },
                },
            )
            .unwrap();
        }
        fn view(chat: &mut WorldInstance) -> String {
            String::from_utf8(chat.view().unwrap()).unwrap()
        }

        assert_eq!(view(&mut chat), "chat seen=0 last= input=");
        // Type "hi", then enter: a chat.say request goes out as r0.
        send(&mut chat, b"h");
        send(&mut chat, b"i");
        let typing = view(&mut chat);
        assert!(typing.contains("input=hi"), "{typing}");
        send(&mut chat, b"\r");
        let document = wants_of(&mut chat);
        assert!(
            document.contains("chat.say") && document.contains("hi") && document.contains("\"r0\""),
            "{document}"
        );
        // The say is answered and retired; `hist` asks for history as r1.
        host(
            &mut chat,
            serde_json::json!({"cap":"result","id":"r0","ok":true,"value":{"sequence":1}}),
        );
        assert!(!wants_of(&mut chat).contains("chat.say"));
        send(&mut chat, b"hist");
        send(&mut chat, b"\r");
        let document = wants_of(&mut chat);
        assert!(
            document.contains("chat.history") && document.contains("\"r1\""),
            "{document}"
        );
        // The history answer names the newest line; the view reports it.
        host(
            &mut chat,
            serde_json::json!({
                "cap": "result",
                "id": "r1",
                "ok": true,
                "value": {"lines": [{"sequence": 1, "participant": 1, "text": "hi"}]},
            }),
        );
        assert_eq!(view(&mut chat), "chat seen=1 last=hi input=");
        // The snapshot keeps the seen count and the counter.
        let snapshot = chat.snapshot().unwrap().unwrap();
        let mut restored = runner.instantiate(7).unwrap();
        restored.restore(&snapshot).unwrap();
        assert_eq!(view(&mut restored), "chat seen=1 last=hi input=");
    }

    #[test]
    fn checked_in_roc_screen_reflows_for_a_wider_viewport() {
        let wasm = include_bytes!("../examples/roc-screen/roc-screen.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        let width_of = |bytes: Vec<u8>| -> u16 {
            let text = String::from_utf8(bytes).unwrap();
            let Some(crate::world_presentation::WorldSurface::Screen(screen)) =
                crate::world_presentation::parse_surface(&text).unwrap()
            else {
                panic!("roc-screen must return a structured screen: {text}");
            };
            screen.width
        };
        // The fixed view stays 20 wide; the layout path reflows to the viewport.
        assert_eq!(width_of(live.view().unwrap()), 20);
        assert_eq!(width_of(live.layout_at(40, 5).unwrap()), 30);
        assert_eq!(width_of(live.layout_at(24, 5).unwrap()), 20);
        // Reflow never changes the model or its view.
        assert_eq!(width_of(live.view().unwrap()), 20);
    }

    #[test]
    fn bytes_per_frame_baseline_for_checked_in_examples() {
        // F11.9 baseline: the full-screen frame size each checked-in example
        // produces at rest (seed 7), at the two reference viewports. These
        // numbers are the baseline Phase 12 delta frames (F12.11) must beat;
        // the committed table lives in
        // doc/2026-10-27T00-00-00Z_baseline_bytes_per_frame.md.
        // Rerun with:
        //   cd pkgs/id && cargo test --lib bytes_per_frame_baseline -- --nocapture
        const EXAMPLES: [(&str, &[u8]); 7] = [
            ("lounge", include_bytes!("../examples/lounge/lounge.wasm")),
            (
                "roc-chat",
                include_bytes!("../examples/roc-chat/roc-chat.wasm"),
            ),
            (
                "roc-counter",
                include_bytes!("../examples/roc-counter/counter.wasm"),
            ),
            (
                "roc-grid",
                include_bytes!("../examples/roc-grid/roc-grid.wasm"),
            ),
            (
                "roc-screen",
                include_bytes!("../examples/roc-screen/roc-screen.wasm"),
            ),
            (
                "territory",
                include_bytes!("../examples/territory/territory.wasm"),
            ),
            (
                "tic-tac-toe",
                include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm"),
            ),
        ];
        println!("| example | plaza_layout | bytes @ 80x24 | bytes @ 200x60 |");
        for (name, wasm) in EXAMPLES {
            let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
            let mut live = runner.instantiate(7).unwrap();
            let has_layout = live.layout.is_some();
            let small = live.layout_at(80, 24).unwrap();
            let large = live.layout_at(200, 60).unwrap();
            println!(
                "| {name} | {} | {} | {} |",
                if has_layout {
                    "yes"
                } else {
                    "no (view fallback)"
                },
                small.len(),
                large.len()
            );
            if !has_layout {
                // Without `plaza_layout`, layout_at falls back to view():
                // the frame is the same bytes at both viewports.
                assert_eq!(small, large, "{name}: view fallback must not reflow");
                assert_eq!(small, live.view().unwrap(), "{name}: fallback is view()");
            }
        }
    }

    #[test]
    fn checked_in_territory_renders_a_styled_screen_within_the_default_fuel() {
        let wasm = include_bytes!("../examples/territory/territory.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        live.update(1, b"l ").unwrap();
        let view = String::from_utf8(live.view().unwrap()).unwrap();
        let surface = crate::world_presentation::parse_surface(&view).unwrap();
        let Some(crate::world_presentation::WorldSurface::Screen(screen)) = surface else {
            panic!("territory must return a structured screen: {view}");
        };
        assert_eq!((screen.width, screen.height), (24, 19));
        assert!(
            screen.cells.iter().any(|cell| cell.fg != 0 && cell.bold),
            "the cursor cell must carry a visible style"
        );
    }

    #[test]
    fn checked_in_territory_plays_persists_and_rejoins() {
        let wasm = include_bytes!("../examples/territory/territory.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let joined = |id: u64, name: &str| {
            format!(
                r#"{{"cap":"players","event":"joined","participant":{{"id":{id},"name":"{name}"}}}}"#
            )
        };
        let left = |id: u64, name: &str| {
            format!(
                r#"{{"cap":"players","event":"left","participant":{{"id":{id},"name":"{name}"}}}}"#
            )
        };
        let records = |world: &mut WorldInstance| {
            String::from_utf8(world.records().unwrap().unwrap()).unwrap()
        };
        let grid = |marks: &[(usize, char)]| {
            let mut cells = vec!['.'; 128];
            for &(index, mark) in marks {
                cells[index] = mark;
            }
            cells.into_iter().collect::<String>()
        };

        let mut live = runner.instantiate(7).unwrap();
        assert_eq!(
            live.wants().unwrap().unwrap(),
            br#"{"v":1,"subscribe":["players"],"requests":[]}"#
        );
        let cells = grid(&[(0, 'B'), (1, 'A'), (16, 'B'), (17, 'A')]);
        live.update(0, joined(1, "ann").as_bytes()).unwrap();
        live.update(1, b"l ").unwrap();
        live.update(0, joined(2, "bob").as_bytes()).unwrap();
        live.update(2, b" ").unwrap();
        live.update(2, b"l ").unwrap();
        live.update(2, b"hj ").unwrap();
        live.update(1, b"j ").unwrap();
        assert_eq!(
            records(&mut live),
            format!(
                r#"{{"width":16,"height":8,"cells":"{cells}","seats":[{{"mark":"A","name":"ann","cells":2,"online":true,"x":1,"y":1}},{{"mark":"B","name":"bob","cells":2,"online":true,"x":0,"y":1}}]}}"#
            )
        );

        let view = String::from_utf8(live.view().unwrap()).unwrap();
        let surface = crate::world_presentation::parse_surface(&view).unwrap();
        let Some(crate::world_presentation::WorldSurface::Screen(screen)) = surface else {
            panic!("territory must return a structured screen: {view}");
        };
        let rendered: Vec<String> = screen
            .cells
            .chunks(usize::from(screen.width))
            .map(|row| row.iter().map(|cell| cell.symbol.as_str()).collect())
            .collect();
        let text = rendered.join("\n");
        assert!(text.contains("free=124"), "{text}");
        assert!(
            text.contains("A ann 2 online") && text.contains("B bob 2 online"),
            "{text}"
        );
        assert!(!view.contains('\u{1b}'), "{view}");

        let snapshot = String::from_utf8(live.snapshot().unwrap().unwrap()).unwrap();
        assert_eq!(snapshot, format!("{cells}\nann\nbob"));

        live.update(0, left(1, "ann").as_bytes()).unwrap();
        assert!(
            records(&mut live)
                .contains(r#"{"mark":"A","name":"ann","cells":2,"online":false,"x":0,"y":0}"#)
        );

        let mut restored = runner.instantiate(7).unwrap();
        restored.restore(snapshot.as_bytes()).unwrap();
        assert_eq!(
            records(&mut restored),
            format!(
                r#"{{"width":16,"height":8,"cells":"{cells}","seats":[{{"mark":"A","name":"ann","cells":2,"online":false,"x":0,"y":0}},{{"mark":"B","name":"bob","cells":2,"online":false,"x":0,"y":0}}]}}"#
            )
        );
        restored.update(0, joined(3, "bob").as_bytes()).unwrap();
        restored.update(3, b"jj ").unwrap();
        let cells = grid(&[(0, 'B'), (1, 'A'), (16, 'B'), (17, 'A'), (32, 'B')]);
        assert_eq!(
            records(&mut restored),
            format!(
                r#"{{"width":16,"height":8,"cells":"{cells}","seats":[{{"mark":"A","name":"ann","cells":2,"online":false,"x":0,"y":0}},{{"mark":"B","name":"bob","cells":3,"online":true,"x":0,"y":2}}]}}"#
            )
        );
        assert_eq!(
            String::from_utf8(restored.snapshot().unwrap().unwrap()).unwrap(),
            format!("{cells}\nann\nbob")
        );

        let mut fresh = runner.instantiate(7).unwrap();
        fresh.update(9, b"l ").unwrap();
        assert!(
            records(&mut fresh)
                .contains(r#"{"mark":"A","name":"p9","cells":1,"online":true,"x":1,"y":0}"#)
        );
    }

    #[test]
    fn wasm_actor_call_order_records_update_records_view() {
        use crate::world::{WorldEvent, WorldEventKind};
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut program = runner.instantiate(7).unwrap();
        let view =
            |program: &mut WorldInstance| String::from_utf8(program.view().unwrap()).unwrap();
        let _ = crate::world::WorldProgram::records(&mut program).unwrap();
        crate::world::WorldProgram::update(
            &mut program,
            &WorldEvent {
                sequence: 1,
                participant_id: 1,
                kind: WorldEventKind::Input(b"inc".to_vec()),
            },
        )
        .unwrap();
        let _ = crate::world::WorldProgram::records(&mut program).unwrap();
        assert!(view(&mut program).contains("count=1"), "wasm handles the actor order");
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
            document.contains("Welcome, ann script!") && !document.contains('<'),
            "the name is sanitized into the JSON: {document}"
        );

        // The world has not granted chat.say: the refusal is data the
        // program sees, and the request is retired.
        let denied = serde_json::json!({
            "cap": "result",
            "id": "r0",
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
            "id": "r1",
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
                .view()
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
                .view()
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
              (func (export "plaza_view") (param i32) (result i32) (i32.const 0)
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
              (func (export "plaza_view") (param i32) (result i32) (i32.const 0))
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
        assert!(world.view().unwrap_err().to_string().contains("poisoned"));
    }

    #[test]
    fn metrics_count_each_call_kind_and_track_max_and_last() {
        let runner = compile(ECHO, SandboxLimits::default()).unwrap();
        let mut world = runner.instantiate(0).unwrap();
        // Instantiation already ran `plaza_init`; nothing else has run.
        assert_eq!(world.metrics().get(GuestCall::Init).calls, 1);
        assert!(world.last_fuel(GuestCall::Init) > 0);
        assert_eq!(world.metrics().get(GuestCall::View), CallStats::default());

        world.view().unwrap();
        world.view().unwrap();
        world.update(1, b"event").unwrap();
        let view = world.metrics().get(GuestCall::View);
        assert_eq!(view.calls, 2);
        assert_eq!(view.fuel_total, view.fuel_last * 2);
        assert_eq!(view.fuel_max, view.fuel_last);
        assert!(view.fuel_last > 0);
        assert_eq!(world.metrics().get(GuestCall::Update).calls, 1);
        // Optional exports that are absent never reach the guest.
        assert_eq!(world.records().unwrap(), None);
        assert_eq!(world.metrics().get(GuestCall::Records).calls, 0);
        assert_eq!(
            world
                .metrics()
                .iter()
                .map(|(call, _)| call)
                .collect::<Vec<_>>(),
            GuestCall::ALL
        );
    }

    #[test]
    fn metrics_record_the_full_limit_for_a_call_that_exhausted_its_fuel() {
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
        let mut world = compile(
            loop_module,
            SandboxLimits {
                fuel: 1_000,
                ..SandboxLimits::default()
            },
        )
        .unwrap()
        .instantiate(0)
        .unwrap();
        world.view().unwrap_err();
        assert_eq!(world.last_fuel(GuestCall::View), 1_000);
        // A trapped view no longer poisons the instance (F11.17): the retry
        // runs the guest again, so the call is counted a second time.
        world.view().unwrap_err();
        assert_eq!(world.metrics().get(GuestCall::View).calls, 2);
    }

    #[test]
    fn metrics_survive_a_trap_with_guest_diagnostics() {
        // The diagnostic getters refill the store; the trap's fuel must be
        // read before that.
        let diagnostic_module = r#"
            (module
              (memory (export "memory") 1 1)
              (func (export "plaza_alloc") (param i32) (result i32) (i32.const 0))
              (func (export "plaza_free") (param i32 i32))
              (func (export "plaza_init") (param i64) (result i32) (i32.const 0))
              (func (export "plaza_update") (param i32 i32 i32 i32) (result i32) unreachable)
              (func (export "plaza_view") (param i32) (result i32) (i32.const 0))
              (func (export "plaza_out_len") (result i32) (i32.const 0))
              (func (export "plaza_error_ptr") (result i32) (i32.const 0))
              (func (export "plaza_error_len") (result i32) (i32.const 0)))
        "#;
        let mut world = compile(diagnostic_module, SandboxLimits::default())
            .unwrap()
            .instantiate(0)
            .unwrap();
        world.update(1, b"event").unwrap_err();
        let spent = world.last_fuel(GuestCall::Update);
        assert!(spent > 0 && spent < 1_000, "{spent}");
    }

    /// A checked-in example and the scripted session that exercises it.
    struct Example {
        name: &'static str,
        wasm: &'static [u8],
        drive: fn(&mut WorldInstance),
    }

    fn host_event(world: &mut WorldInstance, json: &str) {
        world.update(0, json.as_bytes()).unwrap();
    }

    fn drive_counter(world: &mut WorldInstance) {
        world.update(1, b"inc").unwrap();
        world.update(1, b"inc").unwrap();
    }

    fn drive_tic_tac_toe(world: &mut WorldInstance) {
        for (participant, cell) in [(1u64, "0"), (2, "3"), (1, "1"), (2, "4"), (1, "2")] {
            world.update(participant, cell.as_bytes()).unwrap();
        }
    }

    fn drive_screen(world: &mut WorldInstance) {
        world.update(1, b"+").unwrap();
        world.update(1, b"+").unwrap();
    }

    fn drive_grid(world: &mut WorldInstance) {
        world.update(1, b"set").unwrap();
    }

    fn drive_chat(world: &mut WorldInstance) {
        for key in [&b"h"[..], b"i", b"\r"] {
            world.update(1, key).unwrap();
        }
        host_event(
            world,
            r#"{"cap":"result","id":"r0","ok":true,"value":{"sequence":1}}"#,
        );
        for key in [&b"h"[..], b"i", b"s", b"t", b"\r"] {
            world.update(1, key).unwrap();
        }
        host_event(
            world,
            r#"{"cap":"result","id":"r1","ok":true,"value":{"lines":[{"sequence":1,"participant":1,"text":"hi"}]}}"#,
        );
    }

    fn drive_lounge(world: &mut WorldInstance) {
        host_event(
            world,
            r#"{"cap":"players","event":"joined","participant":{"id":1,"name":"ann"}}"#,
        );
        host_event(
            world,
            r#"{"cap":"result","id":"r0","ok":false,"error":"cap_denied"}"#,
        );
        host_event(world, r#"{"cap":"time.tick"}"#);
        host_event(
            world,
            r#"{"cap":"result","id":"r2","ok":true,"value":{"now":5000}}"#,
        );
    }

    fn drive_territory(world: &mut WorldInstance) {
        let joined = |id: u64, name: &str| {
            format!(
                r#"{{"cap":"players","event":"joined","participant":{{"id":{id},"name":"{name}"}}}}"#
            )
        };
        host_event(world, &joined(1, "ann"));
        host_event(world, &joined(2, "bob"));
        // The opening of a two-player game: each seat has claimed a few
        // cells. `territory-full` budgets the crowded game.
        for (participant, moves) in [(1u64, "l "), (2, " "), (2, "l "), (2, "hj "), (1, "j ")] {
            world.update(participant, moves.as_bytes()).unwrap();
        }
    }

    /// A crowded territory game: `seats` players join, then each claims
    /// `per_seat` cells along its own row, so every seat is online with a
    /// cursor on the board and the grid is mostly claimed.
    fn crowd_territory(world: &mut WorldInstance, seats: u64, per_seat: usize) {
        for id in 1..=seats {
            host_event(
                world,
                &format!(
                    r#"{{"cap":"players","event":"joined","participant":{{"id":{id},"name":"p{id}"}}}}"#
                ),
            );
        }
        for id in 1..=seats {
            let mut keys = "j".repeat(usize::try_from(id - 1).unwrap());
            keys.push(' ');
            keys.push_str(&"l ".repeat(per_seat.saturating_sub(1)));
            world.update(id, keys.as_bytes()).unwrap();
        }
    }

    /// Territory at its seat limit with most of the board claimed.
    fn drive_territory_full(world: &mut WorldInstance) {
        crowd_territory(world, 8, 14);
    }

    fn examples() -> [Example; 8] {
        [
            Example {
                name: "roc-counter",
                wasm: include_bytes!("../examples/roc-counter/counter.wasm"),
                drive: drive_counter,
            },
            Example {
                name: "tic-tac-toe",
                wasm: include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm"),
                drive: drive_tic_tac_toe,
            },
            Example {
                name: "roc-screen",
                wasm: include_bytes!("../examples/roc-screen/roc-screen.wasm"),
                drive: drive_screen,
            },
            Example {
                name: "roc-grid",
                wasm: include_bytes!("../examples/roc-grid/roc-grid.wasm"),
                drive: drive_grid,
            },
            Example {
                name: "roc-chat",
                wasm: include_bytes!("../examples/roc-chat/roc-chat.wasm"),
                drive: drive_chat,
            },
            Example {
                name: "lounge",
                wasm: include_bytes!("../examples/lounge/lounge.wasm"),
                drive: drive_lounge,
            },
            Example {
                name: "territory",
                wasm: include_bytes!("../examples/territory/territory.wasm"),
                drive: drive_territory,
            },
            Example {
                name: "territory-full",
                wasm: include_bytes!("../examples/territory/territory.wasm"),
                drive: drive_territory_full,
            },
        ]
    }

    /// Worst single-call fuel per call kind for one example's scripted
    /// session: init, the driven updates, then every read at the final state,
    /// and a restore of the snapshot on a fresh instance. Kinds the module
    /// does not export are absent.
    fn measure_example(example: &Example) -> Vec<(GuestCall, u64)> {
        let runner = Sandbox::compile(example.wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        (example.drive)(&mut live);
        live.view().unwrap();
        live.layout_at(40, 20).unwrap();
        live.records().unwrap();
        live.wants().unwrap();
        let snapshot = live.snapshot().unwrap();
        let mut restored = runner.instantiate(7).unwrap();
        if let Some(snapshot) = snapshot {
            restored.restore(&snapshot).unwrap();
        }
        GuestCall::ALL
            .into_iter()
            .filter_map(|call| {
                let max = live
                    .metrics()
                    .get(call)
                    .fuel_max
                    .max(restored.metrics().get(call).fuel_max);
                let ran = live.metrics().get(call).calls + restored.metrics().get(call).calls > 0;
                ran.then_some((call, max))
            })
            .collect()
    }

    /// Baseline plus 25 percent, rounded up to the next `100_000`.
    fn budget_for(baseline: u64) -> u64 {
        (baseline * 5).div_ceil(4).div_ceil(100_000) * 100_000
    }

    /// `1234567` as `1_234_567`, the way the table is written.
    fn grouped(number: u64) -> String {
        let digits = number.to_string();
        let mut out = String::new();
        for (i, digit) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push('_');
            }
            out.push(digit);
        }
        out
    }

    /// Prints the budget table for the current guests, ready to paste into
    /// `FUEL_BUDGETS`:
    /// `cargo test --lib -- --ignored print_fuel_baseline --nocapture`.
    #[test]
    #[ignore = "prints the baseline to re-baseline FUEL_BUDGETS by hand"]
    fn print_fuel_baseline() {
        for example in examples() {
            for (call, fuel) in measure_example(&example) {
                println!(
                    "    (\"{}\", GuestCall::{call:?}, {}), // baseline {fuel}",
                    example.name,
                    grouped(budget_for(fuel))
                );
            }
        }
    }

    /// Per-call fuel budgets: the measured baseline plus 25 percent, rounded
    /// up to the next `100_000`. Regenerate with `print_fuel_baseline`.
    const FUEL_BUDGETS: &[(&str, GuestCall, u64)] = &[
        ("roc-counter", GuestCall::Init, 100_000), // baseline 1215
        ("roc-counter", GuestCall::Update, 100_000), // baseline 3759
        ("roc-counter", GuestCall::View, 100_000), // baseline 1327
        ("roc-counter", GuestCall::Layout, 100_000), // baseline 1337
        ("roc-counter", GuestCall::Records, 100_000), // baseline 1866
        ("roc-counter", GuestCall::Wants, 100_000), // baseline 733
        ("roc-counter", GuestCall::Snapshot, 100_000), // baseline 874
        ("roc-counter", GuestCall::Restore, 100_000), // baseline 3238
        ("tic-tac-toe", GuestCall::Init, 100_000), // baseline 1242
        ("tic-tac-toe", GuestCall::Update, 100_000), // baseline 7839
        ("tic-tac-toe", GuestCall::View, 100_000), // baseline 5329
        ("tic-tac-toe", GuestCall::Layout, 100_000), // baseline 6265
        ("tic-tac-toe", GuestCall::Records, 100_000), // baseline 7347
        ("tic-tac-toe", GuestCall::Wants, 100_000), // baseline 1664
        ("tic-tac-toe", GuestCall::Snapshot, 100_000), // baseline 769
        ("tic-tac-toe", GuestCall::Restore, 100_000), // baseline 14204
        ("roc-screen", GuestCall::Init, 100_000),  // baseline 1573
        ("roc-screen", GuestCall::Update, 100_000), // baseline 13141
        ("roc-screen", GuestCall::View, 200_000),  // baseline 123675
        ("roc-screen", GuestCall::Layout, 500_000), // baseline 378384
        ("roc-screen", GuestCall::Records, 100_000), // baseline 3217
        ("roc-screen", GuestCall::Wants, 100_000), // baseline 904
        ("roc-screen", GuestCall::Snapshot, 100_000), // baseline 1061
        ("roc-screen", GuestCall::Restore, 100_000), // baseline 4129
        ("roc-grid", GuestCall::Init, 100_000),    // baseline 1585
        ("roc-grid", GuestCall::Update, 100_000),  // baseline 6150
        ("roc-grid", GuestCall::View, 500_000),    // baseline 391196
        ("roc-grid", GuestCall::Layout, 500_000),  // baseline 389763
        ("roc-grid", GuestCall::Records, 100_000), // baseline 2232
        ("roc-grid", GuestCall::Wants, 100_000),   // baseline 1082
        ("roc-grid", GuestCall::Snapshot, 100_000), // baseline 1341
        ("roc-grid", GuestCall::Restore, 100_000), // baseline 4953
        ("roc-chat", GuestCall::Init, 100_000),    // baseline 1271
        ("roc-chat", GuestCall::Update, 200_000),  // baseline 89763
        ("roc-chat", GuestCall::View, 100_000),    // baseline 5997
        ("roc-chat", GuestCall::Layout, 100_000),  // baseline 6949
        ("roc-chat", GuestCall::Records, 100_000), // baseline 23438
        ("roc-chat", GuestCall::Wants, 100_000),   // baseline 42766
        ("roc-chat", GuestCall::Snapshot, 100_000), // baseline 4767
        ("roc-chat", GuestCall::Restore, 100_000), // baseline 7884
        ("lounge", GuestCall::Init, 100_000),      // baseline 1271
        ("lounge", GuestCall::Update, 100_000),    // baseline 69255
        ("lounge", GuestCall::View, 100_000),      // baseline 5550
        ("lounge", GuestCall::Layout, 100_000),    // baseline 6502
        ("lounge", GuestCall::Records, 100_000),   // baseline 71278
        ("lounge", GuestCall::Wants, 300_000),     // baseline 194873
        ("lounge", GuestCall::Snapshot, 100_000),  // baseline 70261
        ("lounge", GuestCall::Restore, 100_000),   // baseline 33299
        ("territory", GuestCall::Init, 100_000),   // baseline 1593
        ("territory", GuestCall::Update, 100_000), // baseline 68643
        ("territory", GuestCall::View, 1_500_000), // baseline 1134256
        ("territory", GuestCall::Layout, 1_500_000), // baseline 1133740
        ("territory", GuestCall::Records, 100_000), // baseline 58449
        ("territory", GuestCall::Wants, 100_000),  // baseline 3375
        ("territory", GuestCall::Snapshot, 100_000), // baseline 10713
        ("territory", GuestCall::Restore, 100_000), // baseline 23335
        ("territory-full", GuestCall::Init, 100_000), // baseline 1593
        ("territory-full", GuestCall::Update, 1_100_000), // baseline 831551
        ("territory-full", GuestCall::View, 2_100_000), // baseline 1611425
        ("territory-full", GuestCall::Layout, 2_100_000), // baseline 1612121
        ("territory-full", GuestCall::Records, 600_000), // baseline 436803
        ("territory-full", GuestCall::Wants, 100_000), // baseline 3375
        ("territory-full", GuestCall::Snapshot, 100_000), // baseline 12801
        ("territory-full", GuestCall::Restore, 100_000), // baseline 32317
    ];

    #[test]
    fn territory_view_fits_the_default_fuel_with_every_seat_taken() {
        let wasm = include_bytes!("../examples/territory/territory.wasm");
        let limits = SandboxLimits::default();
        let runner = Sandbox::compile(wasm, limits).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        crowd_territory(&mut live, 8, 14);
        let view = live
            .view()
            .unwrap_or_else(|error| panic!("a full territory view must not trap: {error:#}"));
        let view = String::from_utf8(view).unwrap();
        let surface = crate::world_presentation::parse_surface(&view).unwrap();
        let Some(crate::world_presentation::WorldSurface::Screen(screen)) = surface else {
            panic!("territory must return a structured screen: {view}");
        };
        let rows: Vec<String> = screen
            .cells
            .chunks(usize::from(screen.width))
            .map(|row| row.iter().map(|cell| cell.symbol.as_str()).collect())
            .collect();
        let text = rows.join("\n");
        assert!(text.contains("free=16"), "{text}");
        assert!(text.contains("H p8 14 online"), "{text}");
        // One view may use at most 40 percent of the limit, so a full
        // house leaves room for growth.
        let spent = live.last_fuel(GuestCall::View);
        assert!(
            spent <= limits.fuel * 2 / 5,
            "a full-house view used {spent} of {} fuel",
            limits.fuel
        );
    }

    #[test]
    fn territory_scoreboard_lines_are_cut_at_the_screen_width() {
        let wasm = include_bytes!("../examples/territory/territory.wasm");
        let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
        let mut live = runner.instantiate(7).unwrap();
        host_event(
            &mut live,
            r#"{"cap":"players","event":"joined","participant":{"id":1,"name":"abcdefghijklmnopqrst"}}"#,
        );
        live.update(1, b" ").unwrap();
        let view = String::from_utf8(live.view().unwrap()).unwrap();
        let surface = crate::world_presentation::parse_surface(&view).unwrap();
        let Some(crate::world_presentation::WorldSurface::Screen(screen)) = surface else {
            panic!("territory must return a structured screen: {view}");
        };
        let rows: Vec<String> = screen
            .cells
            .chunks(usize::from(screen.width))
            .map(|row| row.iter().map(|cell| cell.symbol.as_str()).collect())
            .collect();
        // Names keep 16 bytes; "A " + 16 + " 1 " leaves three columns of "online".
        assert_eq!(rows[11], "A abcdefghijklmnop 1 onl", "{rows:?}");
        assert_eq!(rows[0], "+-territory------+      ");
        assert_eq!(rows[9], "+----------------+      ");
        assert_eq!(rows[10], "free=127 [hjkl/space]   ");
        assert_eq!(rows[1], "|a...............|      ");
        assert_eq!(rows[12], " ".repeat(24));
    }

    #[test]
    #[ignore = "prints territory view fuel by seat count"]
    fn print_territory_view_fuel_by_seats() {
        // `TERRITORY_WASM=FILE` measures a variant build instead.
        let wasm = match std::env::var_os("TERRITORY_WASM") {
            Some(path) => std::fs::read(path).unwrap(),
            None => include_bytes!("../examples/territory/territory.wasm").to_vec(),
        };
        let runner = Sandbox::compile(&wasm, SandboxLimits::default()).unwrap();
        for (seats, per_seat) in [(0, 0), (1, 1), (2, 14), (4, 14), (8, 1), (8, 14)] {
            let mut live = runner.instantiate(7).unwrap();
            crowd_territory(&mut live, seats, per_seat);
            let outcome = live.view().map_or("trap", |_| "ok");
            println!(
                "seats={seats} per_seat={per_seat} view={outcome} fuel={}",
                live.last_fuel(GuestCall::View)
            );
        }
    }

    #[test]
    fn checked_in_examples_stay_within_fuel_budgets() {
        let mut failures = Vec::new();
        for example in examples() {
            assert!(
                example.wasm.len() <= 1024 * 1024,
                "{} is {} bytes, over the 1 MiB module budget",
                example.name,
                example.wasm.len()
            );
            for (call, actual) in measure_example(&example) {
                let budget = FUEL_BUDGETS
                    .iter()
                    .find(|(name, kind, _)| *name == example.name && *kind == call)
                    .map(|(_, _, budget)| *budget);
                match budget {
                    Some(budget) if actual <= budget => {}
                    Some(budget) => failures.push(format!(
                        "{} {}: used {actual} fuel, budget {budget}",
                        example.name,
                        call.name()
                    )),
                    None => failures.push(format!(
                        "{} {}: used {actual} fuel, no budget in FUEL_BUDGETS",
                        example.name,
                        call.name()
                    )),
                }
            }
        }
        assert!(
            failures.is_empty(),
            "fuel budgets exceeded:\n{}\nre-baseline deliberately: update the table and say why in the commit",
            failures.join("\n")
        );
    }
}
