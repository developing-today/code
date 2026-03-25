# Testing & CI Overhaul Plan

Date: 2026-03-25

## Overview

Comprehensive overhaul of testing infrastructure, CI pipeline, and nix integration. Goals:
- Make all tests work (including E2E)
- Add `test-all` / `check-all` aggregate commands
- Run `serve_tests` + E2E inside nix sandbox via NixOS VM integration tests
- Add `check-nix` justfile command
- GitHub Actions CI workflow
- Multi-port server support

## Current State

- **Unit tests**: 484 pass (in sandbox via `test-sandbox` which skips `serve_tests`)
- **Integration tests**: 64 total, 53 run in sandbox (11 `serve_tests` skipped — need network for iroh endpoint)
- **Web unit tests**: 113 pass via `bun test`
- **E2E tests**: 15 Playwright tests exist but **untested** (never verified to actually pass)
- **`nix flake check`**: 11 checks pass (all skip `serve_tests`, no E2E)
- **No GitHub CI**: No `.github/workflows/` directory exists
- **No `test-all`**: No aggregate that runs unit + integration + web + e2e
- **No `check-nix`**: No just command for `nix flake check`
- **Multi-port**: `id serve --web 0` already supports random port assignment, but iroh itself binds port 0 for its internal protocol. Multiple servers can run with separate `--data-dir` (via `ID_STORE_PATH` or workdir isolation)

## Tasks

### Task 1: Ensure nix-common installs playwright

**Problem**: `nix-common.nix` has `chromium` and `firefox` and sets `PLAYWRIGHT_*` env vars, but does NOT include `playwright` itself as a package. The e2e tests use `bunx playwright test` which downloads playwright at runtime — this fails in nix sandbox (no network).

**Solution**:
- Add `playwright-driver` (or `playwright-driver.browsers`) from nixpkgs to `nix-common.nix`
- Set `PLAYWRIGHT_BROWSERS_PATH` env var to point to nix-provided browsers
- Alternatively, since we use `bunx playwright`, ensure `@playwright/test` is installed via bun and browsers are provided via nix env vars (current approach may work — need to verify)
- Key: ensure `PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` and `PLAYWRIGHT_FIREFOX_EXECUTABLE_PATH` point to working binaries

**Files**: `nix-common.nix`

### Task 2: Fix and verify E2E tests pass

**Problem**: E2E tests were written but never verified to actually pass. The playwright config starts `../target/debug/id serve --web --port 4173 --ephemeral` as the webServer.

**Steps**:
1. Build web variant: `just build`
2. Run `just test-e2e` and fix any failures
3. Common issues to check:
   - Server startup timing (60s timeout may not be enough on slow builds)
   - Selector accuracy (elements may have changed since tests were written)
   - Port conflicts (4173 might be in use)
   - Playwright browser installation (bun install may need to pull playwright browsers)

**Files**: `e2e/tests/basic.spec.ts`, `e2e/playwright.config.ts`

### Task 3: Add `test-all` and `check-all` recipes

**Problem**: No single command runs ALL tests including E2E.

**Solution**:
- `test-all`: runs `test test-web-unit test-web-typecheck test-e2e` (all tests, including serve_tests and e2e)
- `check-all`: runs `fix ci test-e2e` (or `fix test-all ...`)
- Add corresponding flake apps (not checks — these need network)

**Files**: `justfile`, `flake.nix`

### Task 4: Fix nix flake check — ensure current checks pass

**Problem**: Need to verify `nix flake check` still passes after all changes. May have regressions from build tooling changes.

**Steps**:
1. `git add -A` (ensure all files tracked)
2. `nix flake check --no-build` (evaluation)
3. Build individual checks to find issues
4. Fix any failures

**Files**: `flake.nix`, various

### Task 5: Multi-port server support

**Problem**: User wants to "enable running multiple servers on different ports if possible."

**Current state**: `id serve --web 0` already supports random port assignment. Multiple servers can coexist if they use different data directories (each server creates its own iroh endpoint). The `--ephemeral` flag uses in-memory storage, allowing multiple ephemeral servers.

**Solution**:
- Verify multiple servers can run concurrently with `--web 0 --ephemeral` (integration tests already test this: `test_serve_web_multiple_random_ports`)
- Add `--iroh-port` flag to control the iroh protocol port (currently always 0 = random)
- Document multi-server usage in README/ARCHITECTURE

**Files**: `src/cli.rs`, `src/commands/serve.rs`, documentation

### Task 6: NixOS VM integration tests for serve_tests + E2E

**Problem**: `serve_tests` and E2E tests need network access, which the nix sandbox doesn't provide. Solution: NixOS VM-based integration tests using `pkgs.testers.runNixOSTest` (or `pkgs.nixosTest`).

**Approach** (based on reference articles):
1. Create a NixOS module for `id` service (`nix/id-module.nix`):
   ```nix
   { config, pkgs, lib, ... }:
   let cfg = config.services.id; in {
     options.services.id = {
       enable = lib.mkEnableOption "id file sharing service";
       port = lib.mkOption { type = lib.types.port; default = 3000; };
       ephemeral = lib.mkOption { type = lib.types.bool; default = true; };
       package = lib.mkPackageOption pkgs "id" {};
     };
     config = lib.mkIf cfg.enable {
       systemd.services.id = {
         wantedBy = [ "multi-user.target" ];
         serviceConfig.ExecStart = "${cfg.package}/bin/id serve --web ${toString cfg.port} --ephemeral --no-relay";
       };
     };
   }
   ```

2. Create NixOS test for serve functionality (`nix/tests/serve-test.nix`):
   ```nix
   { name = "id-serve-test";
     nodes.server = { pkgs, ... }: {
       imports = [ ../id-module.nix ];
       services.id = { enable = true; port = 3000; package = id-web-package; };
     };
     testScript = ''
       start_all()
       server.wait_for_unit("id.service")
       server.wait_for_open_port(3000)
       server.succeed("curl -f http://localhost:3000/")
       # Test file creation via API
       server.succeed("curl -f -X POST http://localhost:3000/api/new -H 'Content-Type: application/json' -d '{\"name\":\"test.txt\"}'")
       server.succeed("curl -f http://localhost:3000/ | grep test.txt")
     '';
   }
   ```

3. Create NixOS test for E2E with chromium (`nix/tests/e2e-test.nix`):
   ```nix
   { name = "id-e2e-test";
     nodes.server = { pkgs, ... }: {
       imports = [ ../id-module.nix ];
       services.id = { enable = true; port = 4173; };
       environment.systemPackages = [ pkgs.chromium pkgs.bun ];
       # Copy e2e test files
     };
     testScript = ''
       start_all()
       server.wait_for_unit("id.service")
       server.wait_for_open_port(4173)
       # Run playwright tests inside VM
       server.succeed("cd /e2e && PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH=... bunx playwright test --project=chromium")
     '';
   }
   ```

4. Register in `flake.nix` as checks:
   ```nix
   checks.nixos-serve = pkgs.testers.runNixOSTest (import ./nix/tests/serve-test.nix { inherit id-web-package; });
   checks.nixos-e2e = pkgs.testers.runNixOSTest (import ./nix/tests/e2e-test.nix { ... });
   ```

**Key considerations**:
- NixOS VM tests have full network (loopback), so `serve_tests` equivalent works
- E2E in VM is heavier (needs chromium in VM) — consider chromium-only for CI
- VMs boot fast (~5s) with KVM support
- GitHub Actions supports KVM (nested virtualization)
- These are x86_64-linux only

**Files**: `nix/id-module.nix`, `nix/tests/serve-test.nix`, `nix/tests/e2e-test.nix`, `flake.nix`

### Task 7: `check-nix` just command

**Problem**: No just command to run `nix flake check -L`.

**Solution**:
- Add `check-nix` recipe: `nix flake check -L`
- Add as flake app (NOT as a check — would cause infinite recursion)
- This is the "full nix CI" command

**Files**: `justfile`, `flake.nix`

### Task 8: GitHub Actions CI workflow

**Problem**: No CI exists.

**Solution**: Create `.github/workflows/check.yml`:
```yaml
name: CI
on:
  push:
    branches: [main]
  pull_request:

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: DeterminateSystems/nix-installer-action@main
      - uses: DeterminateSystems/magic-nix-cache-action@main
      - run: nix -L flake check
```

Key points:
- Uses DeterminateSystems nix installer (fast, reliable)
- Magic Nix Cache for caching builds between runs
- KVM is available on GitHub Actions (needed for NixOS VM tests)
- Single `nix -L flake check` runs everything
- Only runs on push to main + PRs

**Files**: `.github/workflows/check.yml`

### Task 9: Final verification

1. `just check-all` passes locally
2. `nix flake check -L` passes (all checks including NixOS VM tests)
3. All tests accounted for: unit, integration, web, e2e, NixOS VM
4. GitHub Actions workflow is syntactically valid

## Execution Order

1. Task 1 (playwright in nix) — prerequisite for e2e
2. Task 2 (fix e2e) — needs to work before we can add to CI
3. Task 5 (multi-port) — may affect serve tests
4. Task 3 (test-all/check-all) — aggregates
5. Task 4 (nix flake check) — verify current state
6. Task 6 (NixOS VM tests) — the big one, needs working e2e + serve
7. Task 7 (check-nix) — simple, add after VM tests
8. Task 8 (GitHub CI) — add last, after everything works
9. Task 9 (final verification)

## References

- [NixOS tests with flakes](https://blakesmith.me/2024/03/11/how-to-run-nixos-tests-flake-edition.html)
- [NixOS integration tests on GitHub](https://nixcademy.com/posts/nixos-integration-test-on-github/)
- [Example repo](https://github.com/tfc/nixos-integration-test-example)
- [NixOS testing framework](https://blog.thalheim.io/2023/01/08/how-to-use-nixos-testing-framework-with-flakes/)
- Key pattern: `pkgs.testers.runNixOSTest ./test.nix` in flake checks
