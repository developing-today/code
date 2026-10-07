//! Minimal, capability-free Wasm runner for world state transitions.
//!
//! The guest ABI follows the Roc/Zig adapter: `plaza_init(i64) -> i32`,
//! `plaza_update(i32, i32, i32) -> i32`, `plaza_view(i32, i32, i32) -> i32`,
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
                        | "plaza_out_len"
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
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, "plaza_update")
            .map_err(|e| {
                anyhow::anyhow!("guest must export `plaza_update(i32, i32, i32) -> i32`: {e}")
            })?;
        let view = instance
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, "plaza_view")
            .map_err(|e| {
                anyhow::anyhow!("guest must export `plaza_view(i32, i32, i32) -> i32`: {e}")
            })?;
        let out_len = instance
            .get_typed_func::<(), i32>(&mut store, "plaza_out_len")
            .map_err(|e| anyhow::anyhow!("guest must export `plaza_out_len() -> i32`: {e}"))?;

        let model = init.call(&mut store, seed.cast_signed()).map_err(|e| {
            anyhow::anyhow!("guest `plaza_init` failed (trap or fuel exhausted): {e}")
        })?;
        Ok(WorldInstance {
            limits: self.limits,
            store,
            memory,
            alloc,
            free,
            update,
            view,
            out_len,
            model,
            poisoned: false,
        })
    }
}

/// Persistent guest state for a single authoritative world.
pub struct WorldInstance {
    limits: SandboxLimits,
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    update: TypedFunc<(i32, i32, i32), i32>,
    view: TypedFunc<(i32, i32, i32), i32>,
    out_len: TypedFunc<(), i32>,
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
    /// Apply one event. The model pointer is committed only if guest update succeeds.
    pub fn update(&mut self, event: &[u8]) -> Result<()> {
        ensure!(
            !self.poisoned,
            "world guest is poisoned after a previous failure"
        );
        self.check_message_size(event)?;
        let result = self.update_inner(event);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn update_inner(&mut self, event: &[u8]) -> Result<()> {
        self.store
            .set_fuel(self.limits.fuel)
            .map_err(|e| anyhow::anyhow!("reset guest fuel: {e}"))?;
        let (ptr, len) = self.copy_input(event)?;
        let result = self.update.call(&mut self.store, (self.model, ptr, len));
        let model = match result {
            Ok(model) => model,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "guest `plaza_update` failed (trap or fuel exhausted): {e}"
                ));
            }
        };
        self.free_input(ptr, len)
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
        let (ptr, len) = self.copy_input(viewer)?;
        let rendered = self.view.call(&mut self.store, (self.model, ptr, len));
        let output_ptr = match rendered {
            Ok(ptr) => ptr,
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "guest `plaza_view` failed (trap or fuel exhausted): {e}"
                ));
            }
        };
        self.free_input(ptr, len)
            .map_err(|e| anyhow::anyhow!("free guest viewer buffer: {e}"))?;
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

    fn copy_input(&mut self, input: &[u8]) -> Result<(i32, i32)> {
        if input.is_empty() {
            return Ok((0, 0));
        }
        let len = i32::try_from(input.len()).context("guest input does not fit wasm32 ABI")?;
        let ptr = self
            .alloc
            .call(&mut self.store, len)
            .map_err(|e| anyhow::anyhow!("guest allocation failed: {e}"))?;
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
        Ok((ptr, len))
    }

    fn free_input(&mut self, ptr: i32, len: i32) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        self.free
            .call(&mut self.store, (ptr, len))
            .map_err(|e| anyhow::anyhow!("free guest input buffer: {e}"))
    }
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
          (func (export "plaza_update") (param i32 i32 i32) (result i32)
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
        world.update(b"event").unwrap();
        assert_eq!(world.view(b"viewer").unwrap(), b"hello");
    }

    #[test]
    fn checked_in_roc_counter_compiles_and_initializes_in_the_import_free_sandbox() {
        let wasm = include_bytes!("../examples/roc-counter/counter.wasm");
        let runner = Sandbox::compile(
            wasm,
            SandboxLimits {
                fuel: 1_000_000_000,
                memory_bytes: 64 * 1024 * 1024,
                ..SandboxLimits::default()
            },
        )
        .unwrap();
        let _world = runner.instantiate(7).unwrap();
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
              (func (export "plaza_update") (param i32 i32 i32) (result i32) (i32.const 0))
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
              (func (export "plaza_update") (param i32 i32 i32) (result i32)
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
        let error = world.update(b"event").unwrap_err().to_string();
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
