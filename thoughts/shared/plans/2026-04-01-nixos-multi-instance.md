# NixOS Multi-Instance Module Implementation Plan

**Goal:** Convert `id-module.nix` from single-instance to `services.id.instances.<name>` pattern and update all 3 test files to use dual instances with isolation assertions.

**Architecture:** The module uses `types.attrsOf (types.submodule ...)` to define per-instance options. Each enabled instance generates its own systemd service (`id-<name>.service`) with isolated WorkingDirectory. No backward compatibility layer — all 3 consumers update atomically.

**Design:** `thoughts/shared/designs/2026-04-01-nixos-multi-instance-design.md`

---

## Dependency Graph

```
Batch 1 (sequential - 1 implementer): 1.1 [module refactor - foundation]
Batch 2 (parallel - 3 implementers): 2.1, 2.2, 2.3 [test file updates - all depend on 1.1]
```

NOTE: Only 2 batches because there are only 4 files and all 3 tests depend on the module.

---

## Batch 1: Module Refactor (1 implementer)

### Task 1.1: Refactor `nix/id-module.nix` to instances pattern
**File:** `pkgs/id/nix/id-module.nix`
**Test:** none (verified by `nix eval` in Batch 2 tasks, and end-to-end by all test files)
**Depends:** none

**What changed:** The entire module is rewritten. `services.id.enable`, `services.id.web`, `services.id.port`, etc. are removed. Replaced with `services.id.package` (shared) and `services.id.instances.<name>.{enable, web, port, ...}` (per-instance submodule). The `config` block uses `lib.mapAttrs'` to generate one systemd service per enabled instance. Firewall collects ports from all `openFirewall` instances.

**Implementation:**

```nix
# NixOS module for the `id` peer-to-peer file sharing service.
#
# Multi-instance module: each entry in `services.id.instances` creates an
# isolated systemd service with its own working directory.
#
# Usage in a NixOS configuration:
#   imports = [ ./nix/id-module.nix ];
#   services.id = {
#     package = idPackage;
#     instances.primary = {
#       enable = true;
#       web = true;
#       port = 3000;
#       ephemeral = true;
#     };
#     instances.secondary = {
#       enable = true;
#       web = true;
#       port = 3001;
#       ephemeral = true;
#     };
#   };
{ config, lib, ... }:
let
  cfg = config.services.id;

  instanceModule = { name, ... }: {
    options = {
      enable = lib.mkEnableOption "this id instance";

      web = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Enable the web interface.";
      };

      port = lib.mkOption {
        type = lib.types.port;
        default = 3000;
        description = "Port for the web interface.";
      };

      irohPort = lib.mkOption {
        type = lib.types.port;
        default = 0;
        description = "Port for the Iroh QUIC endpoint. 0 = random.";
      };

      ephemeral = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Use in-memory storage (content lost on restart).";
      };

      noRelay = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Disable relay servers, use direct connections only.";
      };

      noGossip = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Disable gossip-based peer discovery.";
      };

      noMdns = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Disable mDNS local peer discovery.";
      };

      extraArgs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        description = "Extra command-line arguments passed to `id serve`.";
      };

      openFirewall = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Open the web port in the firewall.";
      };
    };
  };

  # Filter to only enabled instances
  enabledInstances = lib.filterAttrs (_name: icfg: icfg.enable) cfg.instances;

  # Build a systemd service for a single instance
  mkService = name: icfg: {
    description = "id peer-to-peer file sharing service (${name})";
    wantedBy = [ "multi-user.target" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];

    serviceConfig = {
      ExecStart = lib.concatStringsSep " " (
        [
          "${cfg.package}/bin/id"
          "serve"
        ]
        ++ lib.optionals icfg.web [
          "--web"
          "--port"
          (builtins.toString icfg.port)
        ]
        ++ lib.optionals (icfg.irohPort != 0) [
          "--iroh-port"
          (builtins.toString icfg.irohPort)
        ]
        ++ lib.optional icfg.ephemeral "--ephemeral"
        ++ lib.optional icfg.noRelay "--no-relay"
        ++ lib.optional icfg.noGossip "--no-gossip"
        ++ lib.optional icfg.noMdns "--no-mdns"
        ++ icfg.extraArgs
      );
      Restart = "on-failure";
      RestartSec = 5;

      # Hardening
      DynamicUser = true;
      StateDirectory = lib.mkIf (!icfg.ephemeral) "id-${name}";
      RuntimeDirectory = lib.mkIf icfg.ephemeral "id-${name}";
      WorkingDirectory = if icfg.ephemeral then "/run/id-${name}" else "/var/lib/id-${name}";
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = true;
      NoNewPrivileges = true;
    };
  };

  # Collect firewall ports from instances with openFirewall + web enabled
  firewallPorts = lib.concatMap
    (_name: icfg:
      if icfg.openFirewall && icfg.web then [ icfg.port ] else [ ]
    )
    (lib.attrsToList enabledInstances);

in
{
  options.services.id = {
    package = lib.mkOption {
      type = lib.types.package;
      description = "The id package to use (shared by all instances).";
    };

    instances = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule instanceModule);
      default = { };
      description = "Set of id service instances to run.";
    };
  };

  config = lib.mkIf (enabledInstances != { }) {
    systemd.services = lib.mapAttrs'
      (name: icfg: lib.nameValuePair "id-${name}" (mkService name icfg))
      enabledInstances;

    networking.firewall.allowedTCPPorts = lib.mkIf (firewallPorts != [ ]) firewallPorts;
  };
}
```

**Verify:** From the repo root worktree (`/home/user/.local/share/opencode/worktree/code/e2e-nix`), run:
```bash
nix eval .#checks.x86_64-linux --apply 'x: builtins.attrNames x' 2>&1
```
This should succeed without evaluation errors. The actual test names will change (services reference `id-primary` etc.) but evaluation should not fail since `nix eval` just checks the attribute set structure.

**Commit:** `feat(nix): refactor id-module.nix to multi-instance pattern`

---

## Batch 2: Test File Updates (parallel — 3 implementers)

All tasks in this batch depend on Task 1.1 completing first.

### Task 2.1: Update `nix/tests/serve-test.nix` — Dual HTTP API test
**File:** `pkgs/id/nix/tests/serve-test.nix`
**Test:** none (this IS the NixOS VM test — verified by `nix build .#checks.x86_64-linux.nixos-serve`)
**Depends:** 1.1

**What changed:** Module config switches from `services.id = { enable = true; ... }` to `services.id.package` + `services.id.instances.primary` + `services.id.instances.secondary`. Test script gains a `run_api_tests(port)` Python helper that encapsulates ALL existing API tests. Helper is called for both ports. A new isolation test creates a file on primary and verifies it's absent from secondary.

**Implementation:**

```nix
# NixOS VM integration test for the `id` web server.
#
# Tests HTTP endpoints, file creation via API, and basic web UI rendering.
# Runs two isolated instances (ports 3000 + 3001) to verify multi-instance
# support and data isolation.
#
# Usage:
#   pkgs.testers.runNixOSTest (import ./serve-test.nix { inherit idPackage; })
{ idPackage }:
{
  name = "id-serve";

  nodes.server =
    { pkgs, ... }:
    {
      imports = [ ../id-module.nix ];

      services.id = {
        package = idPackage;
        instances.primary = {
          enable = true;
          web = true;
          port = 3000;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
        instances.secondary = {
          enable = true;
          web = true;
          port = 3001;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
      };

      environment.systemPackages = [ pkgs.curl ];
    };

  globalTimeout = 300;

  testScript = ''
    import json

    start_all()

    # ── Boot & service readiness ──────────────────────────────────────────
    server.wait_for_unit("id-primary.service")
    server.wait_for_unit("id-secondary.service")
    server.wait_for_open_port(3000)
    server.wait_for_open_port(3001)

    def run_api_tests(port):
        """Run the full API test suite against a single instance."""
        BASE = f"http://localhost:{port}"

        # ── Home page renders ─────────────────────────────────────────────
        html = server.succeed(f"curl -sf {BASE}/")
        assert "Files" in html, f"[port {port}] Home page missing 'Files': {html[:200]}"

        # ── Static assets served ──────────────────────────────────────────
        server.succeed(f"curl -sf -o /dev/null {BASE}/assets/manifest.json")

        # ── Create a file via API ─────────────────────────────────────────
        create_resp = server.succeed(
            f"curl -sf -X POST {BASE}/api/new "
            f"-H 'Content-Type: application/json' "
            f"-d '{{\"name\": \"hello.txt\"}}'"
        )
        resp = json.loads(create_resp)
        assert "hash" in resp, f"[port {port}] Create response missing 'hash': {create_resp}"
        assert resp.get("name") == "hello.txt", f"[port {port}] Unexpected name: {resp}"
        file_hash = resp["hash"]

        # ── File appears in list ──────────────────────────────────────────
        list_html = server.succeed(f"curl -sf {BASE}/")
        assert "hello.txt" in list_html, f"[port {port}] Created file not in file list"

        # ── File accessible by name ───────────────────────────────────────
        file_html = server.succeed(f"curl -sf {BASE}/file/hello.txt")
        assert "hello.txt" in file_html, f"[port {port}] File page missing filename"

        # ── File accessible by hash ───────────────────────────────────────
        edit_html = server.succeed(f"curl -sf {BASE}/edit/{file_hash}")
        assert "hello.txt" in edit_html, f"[port {port}] Edit page missing filename"

        # ── Save content via API (ProseMirror doc format) ─────────────────
        pm_doc = {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Hello, NixOS!"}]}]}
        save_body = json.dumps({"doc_id": file_hash, "name": "hello.txt", "doc": pm_doc})
        save_resp = server.succeed(
            f"curl -sf -X POST {BASE}/api/save "
            f"-H 'Content-Type: application/json' "
            f"-d '{save_body}'"
        )
        save = json.loads(save_resp)
        assert "hash" in save, f"[port {port}] Save failed: {save_resp}"
        saved_hash = save["hash"]

        # ── Rename via API ────────────────────────────────────────────────
        rename_resp = server.succeed(
            f"curl -sf -X POST {BASE}/api/rename "
            f"-H 'Content-Type: application/json' "
            f"-d '{{\"name\": \"hello.txt\", \"new_name\": \"renamed.txt\", \"archive\": false}}'"
        )
        rename = json.loads(rename_resp)
        assert rename.get("name") == "renamed.txt", f"[port {port}] Rename failed: {rename_resp}"

        # ── Renamed file accessible ───────────────────────────────────────
        server.succeed(f"curl -sf {BASE}/file/renamed.txt")

        # ── Copy via API ──────────────────────────────────────────────────
        copy_resp = server.succeed(
            f"curl -sf -X POST {BASE}/api/copy "
            f"-H 'Content-Type: application/json' "
            f"-d '{{\"name\": \"renamed.txt\", \"new_name\": \"copy.txt\"}}'"
        )
        copy = json.loads(copy_resp)
        assert copy.get("name") == "copy.txt", f"[port {port}] Copy failed: {copy_resp}"

        # ── Both files exist ──────────────────────────────────────────────
        server.succeed(f"curl -sf {BASE}/file/renamed.txt")
        server.succeed(f"curl -sf {BASE}/file/copy.txt")

        # ── Delete via API ────────────────────────────────────────────────
        delete_resp = server.succeed(
            f"curl -sf -X POST {BASE}/api/delete "
            f"-H 'Content-Type: application/json' "
            f"-d '{{\"name\": \"copy.txt\"}}'"
        )

        # ── Verify saved content via blob endpoint ────────────────────────
        blob_content = server.succeed(f"curl -sf {BASE}/blob/{saved_hash}")
        assert "Hello, NixOS!" in blob_content, f"[port {port}] Blob content mismatch: {blob_content[:200]}"

        # ── Health: server still running after all operations ─────────────
        server.succeed(f"curl -sf {BASE}/")

        return saved_hash

    # ── Run full API tests on both instances ──────────────────────────────
    run_api_tests(3000)
    run_api_tests(3001)

    # ── Isolation test: file on primary must NOT appear on secondary ──────
    # Create a unique file on primary
    iso_resp = server.succeed(
        "curl -sf -X POST http://localhost:3000/api/new "
        "-H 'Content-Type: application/json' "
        "-d '{\"name\": \"isolation-test.txt\"}'"
    )
    iso = json.loads(iso_resp)
    assert iso.get("name") == "isolation-test.txt", f"Isolation file creation failed: {iso_resp}"

    # Verify it exists on primary
    primary_html = server.succeed("curl -sf http://localhost:3000/")
    assert "isolation-test.txt" in primary_html, "Isolation file missing on primary"

    # Verify it does NOT exist on secondary
    secondary_html = server.succeed("curl -sf http://localhost:3001/")
    assert "isolation-test.txt" not in secondary_html, "Isolation FAILED: file leaked to secondary instance"
  '';
}
```

**Verify:** From the repo root worktree:
```bash
cd /home/user/.local/share/opencode/worktree/code/e2e-nix && nix build .#checks.x86_64-linux.nixos-serve --no-link 2>&1
```
(Full VM test — takes a few minutes. For quick syntax check: `nix eval .#checks.x86_64-linux --apply 'x: builtins.attrNames x'`)

**Commit:** `test(nix): update serve-test.nix to dual-instance with isolation`

---

### Task 2.2: Update `nix/tests/e2e-test.nix` — Dual Browser DOM test
**File:** `pkgs/id/nix/tests/e2e-test.nix`
**Test:** none (this IS the NixOS VM test — verified by `nix build .#checks.x86_64-linux.nixos-e2e`)
**Depends:** 1.1

**What changed:** Module config switches to multi-instance pattern. Test script gains a `run_dom_tests(port)` helper. Both instances are tested. Isolation test added: file created via API on primary, chromium dump-dom on secondary verifies absence.

**Implementation:**

```nix
# NixOS VM integration test for browser-level validation.
#
# Uses chromium in headless mode (--dump-dom) to verify the web UI renders
# correctly with JavaScript. Runs two isolated instances (ports 4173 + 4174)
# to validate multi-instance support and data isolation.
#
# This is NOT a full Playwright E2E suite — those run outside the sandbox via
# `just test-e2e`. This test ensures the service works end-to-end in a real
# NixOS environment with systemd management.
#
# Usage:
#   pkgs.testers.runNixOSTest (import ./e2e-test.nix { inherit idPackage; })
{ idPackage }:
{
  name = "id-e2e";

  nodes.server =
    { pkgs, ... }:
    {
      imports = [ ../id-module.nix ];

      services.id = {
        package = idPackage;
        instances.primary = {
          enable = true;
          web = true;
          port = 4173;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
        instances.secondary = {
          enable = true;
          web = true;
          port = 4174;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
      };

      environment.systemPackages = [
        pkgs.curl
        pkgs.chromium
      ];

      # Chromium headless needs some resources
      virtualisation.memorySize = 2048;
      virtualisation.cores = 2;
    };

  globalTimeout = 300; # 5 minutes — chromium startup is slow

  testScript = ''
    import json

    start_all()

    # ── Boot & service readiness ──────────────────────────────────────────
    server.wait_for_unit("id-primary.service")
    server.wait_for_unit("id-secondary.service")
    server.wait_for_open_port(4173)
    server.wait_for_open_port(4174)

    def run_dom_tests(port):
        """Run chromium --dump-dom tests against a single instance."""
        BASE = f"http://localhost:{port}"

        # ── Verify basic HTTP ─────────────────────────────────────────────
        server.succeed(f"curl -sf {BASE}/")

        # ── Chromium can render the home page ─────────────────────────────
        home_dom = server.succeed(
            f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
            f"--timeout=15000 {BASE}/ 2>/dev/null"
        )
        assert "Files" in home_dom, f"[port {port}] Home page DOM missing 'Files': {home_dom[:500]}"
        assert "new-file-name" in home_dom, f"[port {port}] Home page missing new file form"

        # ── Create a file for further tests ───────────────────────────────
        create_resp = server.succeed(
            f"curl -sf -X POST {BASE}/api/new "
            f"-H 'Content-Type: application/json' "
            f"-d '{{\"name\": \"browser-test.txt\"}}'"
        )
        resp = json.loads(create_resp)
        file_hash = resp["hash"]

        # ── Chromium renders the file in the list ─────────────────────────
        list_dom = server.succeed(
            f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
            f"--timeout=15000 {BASE}/ 2>/dev/null"
        )
        assert "browser-test.txt" in list_dom, f"[port {port}] Created file not in rendered list"

        # ── Chromium renders the editor page ──────────────────────────────
        editor_dom = server.succeed(
            f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
            f"--timeout=15000 {BASE}/file/browser-test.txt 2>/dev/null"
        )
        assert "editor" in editor_dom, f"[port {port}] Editor page missing editor element"
        assert "browser-test.txt" in editor_dom, f"[port {port}] Editor page missing filename"

        # ── Chromium renders the editor by hash ───────────────────────────
        edit_dom = server.succeed(
            f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
            f"--timeout=15000 {BASE}/edit/{file_hash} 2>/dev/null"
        )
        assert "editor" in edit_dom, f"[port {port}] Edit-by-hash page missing editor"

        # ── Verify JS-dependent UI elements rendered ──────────────────────
        assert "rename" in editor_dom.lower(), f"[port {port}] Editor missing rename button"
        assert "copy" in editor_dom.lower(), f"[port {port}] Editor missing copy button"

        # ── Verify theme is applied ───────────────────────────────────────
        assert "sneak" in editor_dom, f"[port {port}] Editor missing default theme"

        return file_hash

    # ── Run full DOM tests on both instances ──────────────────────────────
    run_dom_tests(4173)
    run_dom_tests(4174)

    # ── Isolation test: file on primary must NOT appear on secondary ──────
    # Create a unique file on primary
    iso_resp = server.succeed(
        "curl -sf -X POST http://localhost:4173/api/new "
        "-H 'Content-Type: application/json' "
        "-d '{\"name\": \"isolation-dom.txt\"}'"
    )
    iso = json.loads(iso_resp)
    assert iso.get("name") == "isolation-dom.txt", f"Isolation file creation failed: {iso_resp}"

    # Verify it appears in primary's DOM
    primary_dom = server.succeed(
        "chromium --headless --disable-gpu --no-sandbox --dump-dom "
        "--timeout=15000 http://localhost:4173/ 2>/dev/null"
    )
    assert "isolation-dom.txt" in primary_dom, "Isolation file missing from primary DOM"

    # Verify it does NOT appear in secondary's DOM
    secondary_dom = server.succeed(
        "chromium --headless --disable-gpu --no-sandbox --dump-dom "
        "--timeout=15000 http://localhost:4174/ 2>/dev/null"
    )
    assert "isolation-dom.txt" not in secondary_dom, "Isolation FAILED: file leaked to secondary instance DOM"
  '';
}
```

**Verify:** From the repo root worktree:
```bash
cd /home/user/.local/share/opencode/worktree/code/e2e-nix && nix build .#checks.x86_64-linux.nixos-e2e --no-link 2>&1
```

**Commit:** `test(nix): update e2e-test.nix to dual-instance with isolation`

---

### Task 2.3: Update `nix/tests/playwright-e2e-test.nix` — Dual Playwright (4 VMs)
**File:** `pkgs/id/nix/tests/playwright-e2e-test.nix`
**Test:** none (this IS the NixOS VM test — verified by `nix build .#checks.x86_64-linux.playwright-e2e`)
**Depends:** 1.1

**What changed:** Each server VM now runs 2 instances. `chromium_server` gets `instances.primary` (port 4173) and `instances.secondary` (port 4175). `firefox_server` gets `instances.primary` (port 4174) and `instances.secondary` (port 4176). The `run_playwright` helper now accepts a `run_id` parameter so the copied test runner directory is unique per run (avoids conflicts when the same client runs 2 sequential Playwright invocations). Total of 4 Playwright runs: chromium→4173, chromium→4175, firefox→4174, firefox→4176.

**Design decision:** The `run_playwright` helper copies the e2e runner to `/tmp/e2e-{run_id}` instead of `/tmp/e2e` so that test-results/reports from run 1 don't interfere with run 2 on the same client VM. The `run_id` is a simple string like `"primary"` or `"secondary"`.

**Implementation:**

```nix
# NixOS VM Playwright E2E test — full browser coverage.
#
# Architecture: 4 VMs communicating over a virtual network
#   - chromium_server: id service instances on ports 4173 + 4175
#   - firefox_server:  id service instances on ports 4174 + 4176
#   - chromium_client: Playwright + Chromium tests against chromium_server
#   - firefox_client:  Playwright + Firefox tests against firefox_server
#
# Each client runs the full Playwright suite against BOTH server instances
# (4 total runs: 2 browsers × 2 instances).
#
# Usage:
#   pkgs.testers.runNixOSTest (import ./playwright-e2e-test.nix {
#     inherit idPackage e2eTestRunner playwrightBrowsers;
#   })
{
  idPackage,
  e2eTestRunner,
  playwrightBrowsers,
}:
{
  name = "id-playwright-e2e";

  # ── Server nodes: each runs two isolated id service instances ────────────
  nodes.chromium_server =
    { ... }:
    {
      imports = [ ../id-module.nix ];

      services.id = {
        package = idPackage;
        instances.primary = {
          enable = true;
          web = true;
          port = 4173;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
        instances.secondary = {
          enable = true;
          web = true;
          port = 4175;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
      };
    };

  nodes.firefox_server =
    { ... }:
    {
      imports = [ ../id-module.nix ];

      services.id = {
        package = idPackage;
        instances.primary = {
          enable = true;
          web = true;
          port = 4174;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
        instances.secondary = {
          enable = true;
          web = true;
          port = 4176;
          ephemeral = true;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          openFirewall = true;
        };
      };
    };

  # ── Client nodes: Playwright + browser, tests run here ──────────────────
  nodes.chromium_client =
    { pkgs, ... }:
    {
      environment.systemPackages = [
        pkgs.nodejs
        pkgs.curl
      ];

      # Chromium needs resources for rendering
      virtualisation.memorySize = 4096;
      virtualisation.cores = 2;
    };

  nodes.firefox_client =
    { pkgs, ... }:
    {
      environment.systemPackages = [
        pkgs.nodejs
        pkgs.curl
      ];

      # Firefox needs resources for rendering
      virtualisation.memorySize = 4096;
      virtualisation.cores = 2;
    };

  globalTimeout = 1200; # 20 minutes — doubled: 4 Playwright runs instead of 2

  testScript = ''
    E2E_RUNNER = "${e2eTestRunner}"
    BROWSERS = "${playwrightBrowsers}"

    # ── Wait for all server instances ──────────────────────────────────────
    start_all()

    chromium_server.wait_for_unit("id-primary.service")
    chromium_server.wait_for_unit("id-secondary.service")
    chromium_server.wait_for_open_port(4173)
    chromium_server.wait_for_open_port(4175)
    firefox_server.wait_for_unit("id-primary.service")
    firefox_server.wait_for_unit("id-secondary.service")
    firefox_server.wait_for_open_port(4174)
    firefox_server.wait_for_open_port(4176)

    # ── Verify all servers reachable from client VMs ───────────────────────
    chromium_client.succeed("curl -sf http://chromium_server:4173/")
    chromium_client.succeed("curl -sf http://chromium_server:4175/")
    firefox_client.succeed("curl -sf http://firefox_server:4174/")
    firefox_client.succeed("curl -sf http://firefox_server:4176/")

    # ── Helper: copy test runner to writable dir and run Playwright ────────
    # The e2eTestRunner is a read-only nix store path; Playwright needs to
    # write test-results/ and playwright-report/ in the working directory.
    # Each run uses a unique directory (/tmp/e2e-{run_id}) to avoid conflicts
    # when the same client runs multiple sequential Playwright invocations.
    def run_playwright(client, project, base_url_var, base_url, run_id):
        work_dir = f"/tmp/e2e-{run_id}"
        client.succeed(
            f"cp -r {E2E_RUNNER} {work_dir} && "
            f"chmod -R u+w {work_dir} && "
            f"cd {work_dir} && "
            f"PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 "
            f"PLAYWRIGHT_BROWSERS_PATH={BROWSERS} "
            f"PLAYWRIGHT_VM_TEST=1 "
            f"{base_url_var}={base_url} "
            f"node node_modules/@playwright/test/cli.js test "
            f"--project={project} 2>&1"
        )

    # ── Run Chromium tests against both server instances ───────────────────
    run_playwright(
        chromium_client, "chromium",
        "CHROMIUM_BASE_URL", "http://chromium_server:4173",
        "chromium-primary"
    )
    run_playwright(
        chromium_client, "chromium",
        "CHROMIUM_BASE_URL", "http://chromium_server:4175",
        "chromium-secondary"
    )

    # ── Run Firefox tests against both server instances ────────────────────
    run_playwright(
        firefox_client, "firefox",
        "FIREFOX_BASE_URL", "http://firefox_server:4174",
        "firefox-primary"
    )
    run_playwright(
        firefox_client, "firefox",
        "FIREFOX_BASE_URL", "http://firefox_server:4176",
        "firefox-secondary"
    )
  '';
}
```

**Verify:** From the repo root worktree:
```bash
cd /home/user/.local/share/opencode/worktree/code/e2e-nix && nix build .#checks.x86_64-linux.playwright-e2e --no-link 2>&1
```

**Commit:** `test(nix): update playwright-e2e-test.nix to dual-instance (4 runs)`

---

## Verification Sequence

After all 4 tasks are complete, run these in order:

### 1. Fast evaluation check (seconds)
```bash
cd /home/user/.local/share/opencode/worktree/code/e2e-nix && nix eval .#checks.x86_64-linux --apply 'x: builtins.attrNames x' 2>&1
```
Confirms all nix files parse and evaluate without type errors.

### 2. serve-test (fastest VM test, ~2-3 min)
```bash
cd /home/user/.local/share/opencode/worktree/code/e2e-nix && nix build .#checks.x86_64-linux.nixos-serve --no-link 2>&1
```

### 3. e2e-test (browser DOM test, ~3-5 min)
```bash
cd /home/user/.local/share/opencode/worktree/code/e2e-nix && nix build .#checks.x86_64-linux.nixos-e2e --no-link 2>&1
```

### 4. playwright-e2e (full browser test, ~15-20 min)
```bash
cd /home/user/.local/share/opencode/worktree/code/e2e-nix && nix build .#checks.x86_64-linux.playwright-e2e --no-link 2>&1
```

### 5. Commit all together
```bash
git add pkgs/id/nix/id-module.nix pkgs/id/nix/tests/serve-test.nix pkgs/id/nix/tests/e2e-test.nix pkgs/id/nix/tests/playwright-e2e-test.nix
git commit -m "feat(nix): multi-instance module with dual-instance tests and isolation"
```

---

## Implementation Notes

### Key design decisions made by planner

1. **`firewallPorts` uses `lib.concatMap` + `lib.attrsToList`**: The `lib.attrsToList` converts the attrset to a list of `{name, value}` pairs, then `lib.concatMap` iterates. This is cleaner than `lib.mapAttrsToList` + `lib.flatten` for conditional port collection.

2. **`run_id` parameter in Playwright helper**: Each Playwright run copies to a unique `/tmp/e2e-{run_id}` directory. This prevents the second run on the same client VM from overwriting test-results/reports from the first run.

3. **Playwright timeout doubled to 1200s**: With 4 runs instead of 2, the test needs more time. Original was 600s for 2 runs, so 1200s gives generous headroom.

4. **No `globalTimeout` increase for serve-test**: The API tests are fast even doubled. 300s is generous.

5. **No `virtualisation.memorySize` increase for e2e-test**: 2048MB is enough — chromium is the bottleneck, and running tests sequentially means only one chromium process at a time.

6. **Cleanup of test files between `run_api_tests` calls**: Not needed. Each call to `run_api_tests` creates `hello.txt`, renames to `renamed.txt`, copies to `copy.txt`, deletes `copy.txt`. The second instance starts clean (ephemeral). The `run_dom_tests` similarly creates `browser-test.txt` on each instance independently.

7. **Service unit names**: NixOS generates `id-primary.service` and `id-secondary.service` from `mapAttrs'` with `nameValuePair "id-${name}"`. Test scripts reference these exact names in `wait_for_unit()`.
