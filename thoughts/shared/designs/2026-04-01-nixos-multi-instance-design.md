---
date: 2026-04-01
topic: "NixOS Multi-Instance Module"
status: validated
---

# NixOS Multi-Instance Module Design

## Problem Statement

The current `id-module.nix` supports only a single `id serve` instance per host. We need each NixOS test VM to run 2 instances to prove multi-instance isolation and test the new `--data-dir`/`--new` features in a real deployment.

## Constraints

- Module is internal-only (used in 3 test nix files within this repo)
- No external consumers exist — backward compatibility not required
- All test files update atomically in the same commit
- `integration-test.nix` does NOT use the module (spawns servers from test binary)
- Each instance needs its own systemd service, working directory, and port
- Must preserve existing systemd hardening (DynamicUser, ProtectSystem, etc.)

## Approach

Convert `id-module.nix` from single-instance to **`services.id.instances.<name>`** pattern using `types.attrsOf (types.submodule ...)`. No backward-compatible shorthand — the module is internal-only with exactly 3 consumers, all updated in the same commit.

### Why not backward compat?

The plan originally called for `services.id.enable = true` shorthand that maps to an implicit `instances.default`. This was dropped because:
- Adds ~30% more module code for a mapping layer
- Creates two code paths to test and maintain
- Zero external consumers benefit from it
- All 3 test files are updated in the same PR commit

## Architecture

### Module Structure (`nix/id-module.nix`)

**Top-level shared option:**
- `services.id.package` — the id binary package (shared by all instances)

**Per-instance options** (`services.id.instances.<name>`):
| Option        | Type          | Default | Description                     |
|---------------|---------------|---------|----------------------------------|
| `enable`      | bool          | false   | Enable this instance             |
| `web`         | bool          | true    | Enable web interface             |
| `port`        | port          | 3000    | Web UI port                      |
| `irohPort`    | port          | 0       | QUIC endpoint port (0 = random)  |
| `ephemeral`   | bool          | false   | In-memory storage                |
| `noRelay`     | bool          | false   | Disable relay servers            |
| `noGossip`    | bool          | false   | Disable gossip discovery         |
| `noMdns`      | bool          | false   | Disable mDNS discovery           |
| `extraArgs`   | list of str   | []      | Extra CLI args                   |
| `openFirewall`| bool          | false   | Open TCP port in firewall        |

### systemd Service Generation

Each enabled instance `<name>` creates:

- **Service**: `id-<name>.service`
- **ExecStart**: `<package>/bin/id serve [--web --port N] [--iroh-port N] [flags] [extraArgs]`
- **WorkingDirectory**: `/var/lib/id-<name>` (persistent) or `/run/id-<name>` (ephemeral)
- **StateDirectory**: `id-<name>` (persistent mode)
- **RuntimeDirectory**: `id-<name>` (ephemeral mode)
- **Hardening**: DynamicUser, ProtectSystem=strict, ProtectHome, PrivateTmp, NoNewPrivileges
- **Restart**: on-failure, 5s delay

### Firewall

Collect all ports from instances where `openFirewall = true` and `web = true`, add to `networking.firewall.allowedTCPPorts`.

## Components

### 1. `nix/id-module.nix` — Refactored Module

- `instanceModule`: submodule type with all per-instance options
- `enabledInstances`: filtered attrset of instances where `enable = true`
- `config` block: maps enabledInstances to systemd services + firewall rules
- Uses `lib.mapAttrs'` with `lib.nameValuePair` for service name generation

### 2. `nix/tests/serve-test.nix` — Dual HTTP API Test

- **instances.primary**: port 3000, ephemeral, all discovery off
- **instances.secondary**: port 3001, ephemeral, all discovery off
- Test script waits for both `id-primary.service` and `id-secondary.service`
- ALL existing API tests run against BOTH ports (home page, static assets, file CRUD, save, rename, copy, delete, blob fetch)
- **Isolation test**: create file on primary, verify it does NOT appear on secondary

### 3. `nix/tests/e2e-test.nix` — Dual Browser DOM Test

- **instances.primary**: port 4173
- **instances.secondary**: port 4174
- ALL chromium --dump-dom tests run against both ports
- **Isolation test**: create file on one instance, verify absent from other's DOM

### 4. `nix/tests/playwright-e2e-test.nix` — Dual Playwright (4 VMs)

- **chromium_server**: instances on ports 4173 + 4175
- **firefox_server**: instances on ports 4174 + 4176
- Each client VM runs full Playwright suite against BOTH server instances (2 runs per browser = 4 total runs)
- Approximately doubles the Playwright test time (~20 min total)

### 5. `nix/tests/integration-test.nix` — No Module Changes

Already updated in Phase 1 (removed `--skip test_serve_web`). This test spawns servers from the test binary, not the NixOS module.

## Data Flow

1. NixOS evaluates `id-module.nix` → generates N systemd service units
2. Each service unit starts `id serve` with its own WorkingDirectory
3. `id serve` creates `.iroh-key`, `.iroh-store/`, `.iroh-serve.lock` in CWD
4. JSON lock file contains `node_id`, `pid`, `addrs`, `web_port`
5. Test scripts wait for systemd units + open ports → run assertions
6. Isolation: each instance's CWD is separate → no shared state

## Error Handling

- If any instance fails to start, `wait_for_unit()` will timeout and the test fails
- If ports conflict, the second instance will fail to bind → caught by test timeout
- `nix flake check` catches module evaluation errors before runtime

## Testing Strategy

- **Module correctness**: `nix eval` / `nix flake check` catches type errors and missing options
- **Runtime behavior**: Each test file runs assertions against both instances
- **Isolation**: Every test file includes a dedicated isolation check (file on A absent from B)
- **Playwright coverage**: Full browser interaction suite on both instances proves multi-instance works end-to-end

## Open Questions

None — design is straightforward refactor of an internal module with 3 known consumers.
