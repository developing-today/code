# Multi-Instance Support for `id`

> See [original plan](../../.opencode/plans/) (generated from design at `pkgs/id/thoughts/shared/designs/2026-04-01-nixos-multi-instance-design.md`)

## Overview

This feature adds the ability to run multiple independent `id` instances on the same machine, each with its own data directory, key material, blob store, and lock file. Previously, `id` assumed a single instance per working directory with hardcoded CWD-relative paths.

### Problem

Running multiple `id` servers on the same machine required manually creating separate directories and `cd`-ing into each one. There was no CLI flag support for data isolation, no structured lock file format for programmatic discovery, and the NixOS module only supported a single instance.

### Solution

Three coordinated changes:

1. **JSON lock file** with structured `ServeInfo` for programmatic service discovery
2. **`--data-dir` and `--new` global flags** for CLI-level instance isolation
3. **NixOS multi-instance module** using `services.id.instances.<name>` submodule pattern


## JSON Lock File

The serve lock file (`.iroh-serve.lock`) was upgraded from a line-based text format to structured JSON using `serde_json`.

### Before (line-based)

```
<node_id>
<pid>
<addr1>
<addr2>
...
```

### After (JSON)

```json
{
  "node_id": "abc123...def456",
  "pid": 12345,
  "addrs": ["127.0.0.1:12345", "[::1]:12345"],
  "web_port": 3001
}
```

### ServeInfo struct

Defined in `src/commands/serve.rs`:

- `node_id: String` — public identity of the serve node
- `pid: u32` — process ID for liveness checking
- `addrs: Vec<String>` — socket addresses (as strings for JSON round-tripping)
- `web_port: Option<u16>` — web UI port when `--web` is enabled, `None` otherwise

The struct derives `Serialize` and `Deserialize` (serde). Lock file creation uses `serde_json::to_string_pretty`, and reading uses `serde_json::from_str` with graceful fallback (returns `None` on parse failure).

### Liveness check

`get_serve_info()` reads the lock file, deserializes the JSON, then checks whether the PID is still alive using `libc::kill(pid, 0)` on Unix. Stale lock files are automatically cleaned up.


## CLI Instance Isolation

Two new global flags on the `Cli` struct enable running multiple independent instances.

### `--data-dir <DIR>` (`-d`)

Points all data paths to an arbitrary directory. The directory is created if it doesn't exist. Early in `main()`, `std::env::set_current_dir(data_dir)` is called, which makes all CWD-relative paths (`.iroh-store`, `.iroh-key`, `.iroh-serve.lock`) resolve to the specified directory with zero changes to path logic throughout the codebase.

```bash
# Run two servers with separate data
id --data-dir ./node1 serve --web --port 3001
id --data-dir ./node2 serve --web --port 3002

# Client commands target a specific instance
id --data-dir ./node1 list
id --data-dir ./node2 repl
```

### `--new [NAME]`

Shorthand for `--data-dir .iroh/<name>/` that creates a named (or randomly-named) subdirectory under `.iroh/` in the current working directory.

```bash
# Named instance
id --new=my-dev serve --port 0

# Random name (auto-generated 8 hex chars)
id --new serve
```

**Validation rules:**
- Name must not contain `/`, `\`, or `..` (rejects path traversal)
- Instance directory must not already exist (prevents accidental reuse; use `--data-dir` for existing dirs)
- Conflicts with `--data-dir` (clap enforces mutual exclusivity)

**Implementation note:** `--new` uses `num_args = 0..=1` with `default_missing_value = ""`. When the name is empty, a random 8-hex-char string is generated via `rand::Rng`. Always use `--new=name` (with equals sign) to avoid clap's greedy argument parsing consuming the next positional argument.

### Design decision: CWD trick

Rather than threading a `data_dir: PathBuf` through every function that touches the filesystem, we call `set_current_dir()` once at startup. This is safe because:

- `id` is a single-process binary (no library consumers calling `set_current_dir` concurrently)
- All data paths are already CWD-relative constants (`KEY_FILE`, `STORE_PATH`, `SERVE_LOCK`)
- Zero changes required to existing path logic in `store.rs`, `serve.rs`, `client.rs`, etc.


## NixOS Multi-Instance Module

The NixOS module (`pkgs/id/nix/id-module.nix`) was refactored from a single-instance configuration to a multi-instance pattern using `types.attrsOf (types.submodule ...)`.

### Configuration interface

```nix
services.id = {
  package = idPackage;  # Shared by all instances

  instances.primary = {
    enable = true;
    web = true;
    port = 3000;
    ephemeral = true;
  };

  instances.secondary = {
    enable = true;
    web = true;
    port = 3001;
    ephemeral = true;
  };
};
```

### Instance options

Each instance supports:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable` | bool | — | Enable this instance |
| `web` | bool | `true` | Enable web interface |
| `port` | port | `3000` | Web interface port |
| `irohPort` | port | `0` | QUIC endpoint port (0 = random) |
| `ephemeral` | bool | `false` | In-memory storage |
| `noRelay` | bool | `false` | Disable relay servers |
| `noGossip` | bool | `false` | Disable gossip peer discovery |
| `noMdns` | bool | `false` | Disable mDNS discovery |
| `extraArgs` | list of str | `[]` | Extra CLI arguments |
| `openFirewall` | bool | `false` | Open web port in firewall |

### Systemd service generation

Each enabled instance produces a systemd service named `id-<name>.service` via `lib.mapAttrs'` + `lib.nameValuePair`:

- **Ephemeral instances:** `WorkingDirectory = /run/id-<name>`, `RuntimeDirectory = id-<name>`
- **Persistent instances:** `WorkingDirectory = /var/lib/id-<name>`, `StateDirectory = id-<name>`

Hardening: `DynamicUser`, `ProtectSystem=strict`, `ProtectHome`, `PrivateTmp`, `NoNewPrivileges`.

### Firewall integration

Ports from instances with both `openFirewall = true` and `web = true` are automatically collected via `lib.concatMap` over `lib.attrsToList enabledInstances` and added to `networking.firewall.allowedTCPPorts`.


## NixOS Test Coverage

All three NixOS test files were updated to exercise dual-instance deployments:

### serve-test.nix (HTTP API validation)

- Two instances: `primary` (port 3000) and `secondary` (port 3001)
- Shared `run_api_tests(port)` Python helper runs the full API test suite against each instance
- **Isolation test:** stores a file on primary, verifies it's absent on secondary (separate blob stores)

### e2e-test.nix (browser DOM rendering)

- Two instances: `primary` (port 4173) and `secondary` (port 4174)
- Shared `run_dom_tests(port)` Python helper runs chromium `--dump-dom` assertions against each instance
- **DOM isolation test:** stores a file on primary, verifies the secondary's DOM doesn't list it

### playwright-e2e-test.nix (full interactive browser tests)

- Each server VM runs 2 instances (chromium server: 4173 + 4175, firefox server: 4174 + 4176)
- `run_playwright` helper gains a `run_id` parameter for unique `/tmp/e2e-{run_id}` working directories
- 4 total Playwright runs: `chromium-primary`, `chromium-secondary`, `firefox-primary`, `firefox-secondary`
- Timeout doubled to 1200s for 4 runs per VM


## Rust Integration Tests

11 new tests added to `tests/cli_integration.rs` (96 total):

### `--data-dir` tests (3)
- `test_data_dir_creates_and_uses_directory` — verifies store/key/lock created in specified dir
- `test_data_dir_isolation` — two servers with different data dirs are independent
- `test_data_dir_list_uses_correct_instance` — `id --data-dir X list` reads from X's lock file

### `--new` flag tests (5)
- `test_new_flag_creates_iroh_subdir` — verifies `.iroh/<name>/` directory creation
- `test_new_flag_random_name` — `--new` without name generates 8-hex-char dir
- `test_new_flag_rejects_path_separator` — `/` in name → error
- `test_new_flag_rejects_existing_dir` — duplicate name → error
- `test_new_flag_conflicts_with_data_dir` — mutual exclusivity enforced

### JSON lock file tests (3)
- `test_lock_file_is_json` — lock file parses as valid `ServeInfo` JSON
- `test_lock_file_contains_web_port` — `web_port` present when `--web` used
- `test_lock_file_no_web_port_without_web` — `web_port` is null without `--web`

### Test infrastructure

`ServerHandle` gained `spawn_with_global_args()` which injects global flags (`--data-dir`, `--new=`) before the `serve` subcommand. Background stderr capture thread drains into `Arc<Mutex<String>>` for diagnostic output on failures.


## Files Changed

### Core feature (committed `42fcecac`)
- `src/commands/serve.rs` — `ServeInfo` struct with Serialize/Deserialize, JSON lock file read/write, `web_port` parameter
- `src/cli.rs` — `--data-dir` and `--new` global flags on `Cli` struct
- `src/main.rs` — `set_current_dir` dispatch for `--data-dir` / `--new`, name validation
- `src/commands/client.rs` — updated to use `ServeInfo` JSON parsing
- `Cargo.toml` — added `serde`, `serde_json`, `rand` dependencies

### Justfile recipes (committed `5495d42d`)
- `pkgs/id/justfile` — `serve-new`, `serve-new-web`, `serve-new-lib` recipes

### Integration tests (committed `0814e632`)
- `tests/cli_integration.rs` — 11 new tests, `spawn_with_global_args()` helper

### NixOS module + tests (committed `e2f05339`)
- `nix/id-module.nix` — multi-instance submodule pattern
- `nix/tests/serve-test.nix` — dual-instance API tests
- `nix/tests/e2e-test.nix` — dual-instance DOM tests
- `nix/tests/playwright-e2e-test.nix` — 4-run multi-instance Playwright tests


## Known Issues

### Playwright flaky websocket tests

`websocket.spec.ts` tests 474 ("can save file and content persists") and 638 ("edits from one user appear in other user's editor") are intermittently flaky in NixOS VM environments. This is a pre-existing timing issue unrelated to multi-instance support. 71/73 tests pass consistently; the 2 failures are race conditions in WebSocket message ordering within the VM test sandbox.


## References

- Design document: `pkgs/id/thoughts/shared/designs/2026-04-01-nixos-multi-instance-design.md`
- Implementation plan: `pkgs/id/thoughts/shared/plans/2026-04-01-nixos-multi-instance.md`
- Testing architecture: `doc/2026-03-29T00-00-00Z_reference_testing_architecture/2026-03-29T00-00-00Z_reference_testing_architecture.md`
- Relevant constants in `src/lib.rs`: `KEY_FILE`, `STORE_PATH`, `SERVE_LOCK`, `CLIENT_KEY_FILE`
