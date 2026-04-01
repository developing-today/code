
# Plan: Comprehensive Testing, Flaky Fix, NixOS Multi-Instance, and Documentation

**Branch**: `feature/data-dir` in worktree `/home/user/.local/share/opencode/worktree/code/e2e-nix`  
**Base commits**: `42fcecac` (JSON lock + --new flag), `5495d42d` (justfile recipes + validation), `6dfad839` (merge main)

---

## Phase 1: Fix Flaky `test_serve_web_*` Tests

**Problem**: `test_serve_web_random_port` and `test_serve_web_multiple_random_ports` fail because the server process exits before printing `node:` and `web:` lines to stdout. Currently skipped in NixOS integration tests.

**Root cause investigation**: The `wait_ready_with_web()` method only reads stdout. If the server crashes during startup, stderr has the error but stdout just hits EOF, producing a confusing `"Server never printed node ID"` panic.

**Fix** (in `tests/cli_integration.rs`):
1. Add stderr capture to `ServerHandle` — spawn a thread to drain stderr into a `Arc<Mutex<String>>`
2. In `wait_ready_with_web()`, when stdout EOF is reached without finding node ID, include the captured stderr in the panic message
3. Add a check: if `process.try_wait()` shows the child already exited, fail immediately with exit code + stderr instead of hanging
4. Remove the `--skip test_serve_web` from `integration-test.nix` (line 52) once tests pass reliably

**Files**: `tests/cli_integration.rs`, `nix/tests/integration-test.nix`

---

## Phase 2: New Integration Tests for --new, --data-dir, JSON Lock File

All new tests go in `tests/cli_integration.rs` inside `serve_tests` module. Each uses `TempDir` for isolation.

### 2a. `--data-dir` tests
- **`test_serve_data_dir_basic`**: Pass `--data-dir <tmpdir>/custom`, verify server starts, lock file is in `<tmpdir>/custom/.iroh-serve.lock`
- **`test_serve_data_dir_creates_missing`**: Pass non-existent directory, verify it's created
- **`test_serve_data_dir_isolation`**: Two servers with different `--data-dir` paths → different node IDs, separate lock files

### 2b. `--new` tests
- **`test_new_flag_auto_name`**: Run `id --new= id` (the `id` subcommand, not `serve`) to just get node ID. Verify `.iroh/<hex>/` directory was created with `.iroh-key` inside
- **`test_new_flag_named`**: Run `id --new=test-instance id`. Verify `.iroh/test-instance/` exists
- **`test_new_flag_duplicate_rejects`**: Create `.iroh/dup/` manually, then run `id --new=dup serve`. Verify exit code non-zero, stderr contains "already exists"
- **`test_new_flag_invalid_name`**: Run `id --new=../escape serve`. Verify exit code non-zero, stderr contains "simple identifier"
- **`test_new_and_data_dir_conflict`**: Run `id --new=foo --data-dir ./bar serve`. Verify clap rejects with conflict error

### 2c. JSON lock file tests (extend existing)
- **`test_serve_json_lock_structure`**: Parse lock file, verify all 4 fields: `node_id` (64 hex), `pid` (> 0, matches actual child PID), `addrs` (array of socket addr strings), `web_port` (null when no --web)
- **`test_serve_json_lock_web_port`** (`#[cfg(feature = "web")]`): Start with `--web 0`, verify `web_port` is a number > 1024 in JSON
- **`test_serve_stale_lock_detection`**: Write a fake JSON lock file with a dead PID (e.g. 999999999), then start a server. Verify it overwrites the stale lock with valid info

**Files**: `tests/cli_integration.rs`

---

## Phase 3: NixOS Multi-Instance Module

Convert `id-module.nix` from single-instance to named instances pattern, with backward-compatible shorthand.

### 3a. Refactor `id-module.nix`

**New API** (instances pattern):
```nix
services.id.instances.node1 = { enable = true; port = 3000; ... };
services.id.instances.node2 = { enable = true; port = 3001; ... };
```

**Backward-compatible shorthand** (still works for simple setups):
```nix
services.id = { enable = true; port = 3000; ... };
# Implicitly creates instances.default with these settings
```

Implementation: top-level `services.id.enable`, `.web`, `.port`, etc. options are preserved. When `services.id.enable = true`, they create an implicit `instances.default` entry. The `instances` attrset uses a `submodule` with the same option definitions. Each instance `<name>` creates:
- `systemd.services."id-<name>"` (or just `"id"` for the `default` instance, for backward compat)
- `WorkingDirectory = /var/lib/id-<name>` (persistent) or `/run/id-<name>` (ephemeral)
- `StateDirectory = "id-<name>"` / `RuntimeDirectory = "id-<name>"`
- Firewall opens each instance's port

### 3b. Update test nix files — all tests run against BOTH instances in parallel

**serve-test.nix**: 
```nix
services.id.instances.primary = { enable = true; port = 3000; ephemeral = true; ... };
services.id.instances.secondary = { enable = true; port = 3001; ephemeral = true; ... };
```
Test script: wait for both services, run ALL API tests (create, save, rename, copy, delete) against BOTH ports, verify complete isolation (file created on primary is NOT on secondary).

**e2e-test.nix**:
```nix  
services.id.instances.primary = { port = 4173; ... };
services.id.instances.secondary = { port = 4174; ... };
```
Run ALL chromium DOM tests against both ports. Verify both UIs render independently, file created on primary is NOT visible on secondary.

**playwright-e2e-test.nix**: Each server VM gets 2 instances:
- `chromium_server`: instances on ports 4173 + 4175
- `firefox_server`: instances on ports 4174 + 4176

Each client VM runs the full Playwright test suite against BOTH instances of its target server (2 full runs per browser). This doubles browser coverage and proves multi-instance isolation under real browser interaction.

**integration-test.nix**: No module changes needed (tests spawn their own servers). Remove `--skip test_serve_web` once Phase 1 is done.

**Files**: `nix/id-module.nix`, `nix/tests/serve-test.nix`, `nix/tests/e2e-test.nix`, `nix/tests/playwright-e2e-test.nix`, `nix/tests/integration-test.nix`

---

## Phase 4: Documentation

Per the DOCUMENTATION_PROTOCOL, create:

**`docs/<datetime>_feature_multi_instance/`** with a comprehensive doc covering:

1. **JSON Lock File Protocol**: format, fields, stale detection via PID, backward compat notes
2. **`--new` and `--data-dir` flags**: usage, when to use each, directory layout (`.iroh/<name>/`), validation rules
3. **Multi-Instance Architecture**: how CWD-relative paths enable isolation, NixOS module instances pattern, systemd service per instance, backward-compatible shorthand
4. **Web Port Discovery**: how `web_port` in JSON lock enables programmatic port discovery with `--port 0`
5. **Justfile recipes**: `serve-new`, `serve-new-web`, `serve-new-lib` — the `--new=` equals-sign trick
6. References section linking to the plan file

**Files**: `docs/<datetime>_feature_multi_instance/<datetime>_feature_multi_instance.md`

---

## Execution Order

1. **Phase 1** first (fix flaky tests — unblocks Phase 3 integration-test.nix change)
2. **Phase 2** (new integration tests — independent of NixOS changes)
3. **Phase 3** (NixOS module refactor + test updates)
4. **Phase 4** (docs — written last so they reflect final state)

**Verification after each phase**: `just check` for Rust changes, `nix flake check` for NixOS changes (or at least `nix eval` for syntax).

---

## Risks & Notes

- **Phase 3 is the most complex**: The instances refactor changes the NixOS module API, but backward compat is preserved. All 4 test files must update atomically. `nix flake check` will catch mismatches.
- **Playwright double-run adds VM time**: Running all tests against 2 instances per browser doubles the test time (~20 min total). This is acceptable since it's a VM-based CI check, not a fast-feedback loop.
- **Flaky test root cause is uncertain**: Phase 1 starts with investigation. If the root cause is a legitimate race in the binary (not the test), we may need to fix `serve.rs` too.
- **Don't kill port 3000 server**: All work happens in the worktree, and test servers use ephemeral ports.
