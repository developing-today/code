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
