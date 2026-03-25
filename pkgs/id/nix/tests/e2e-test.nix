# NixOS VM integration test for browser-level validation.
#
# Uses chromium in headless mode (--dump-dom) to verify the web UI renders
# correctly with JavaScript. This validates the full stack: systemd → iroh →
# axum → embedded assets → browser DOM rendering.
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
        enable = true;
        package = idPackage;
        web = true;
        port = 4173;
        ephemeral = true;
        noRelay = true;
        noGossip = true;
        noMdns = true;
        openFirewall = true;
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

    PORT = 4173
    BASE = f"http://localhost:{PORT}"

    start_all()

    # ── Boot & service readiness ──────────────────────────────────────────
    server.wait_for_unit("id.service")
    server.wait_for_open_port(PORT)

    # ── Verify basic HTTP ─────────────────────────────────────────────────
    server.succeed(f"curl -sf {BASE}/")

    # ── Chromium can render the home page ─────────────────────────────────
    # --dump-dom renders the page (including JS execution) and prints the DOM
    home_dom = server.succeed(
        f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
        f"--timeout=15000 {BASE}/ 2>/dev/null"
    )
    assert "Files" in home_dom, f"Home page DOM missing 'Files': {home_dom[:500]}"
    assert "new-file-name" in home_dom, "Home page missing new file form"

    # ── Create a file for further tests ───────────────────────────────────
    create_resp = server.succeed(
        f"curl -sf -X POST {BASE}/api/new "
        f"-H 'Content-Type: application/json' "
        f"-d '{{\"name\": \"browser-test.txt\"}}'"
    )
    resp = json.loads(create_resp)
    file_hash = resp["hash"]

    # ── Chromium renders the file in the list ─────────────────────────────
    list_dom = server.succeed(
        f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
        f"--timeout=15000 {BASE}/ 2>/dev/null"
    )
    assert "browser-test.txt" in list_dom, "Created file not in rendered list"

    # ── Chromium renders the editor page ──────────────────────────────────
    editor_dom = server.succeed(
        f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
        f"--timeout=15000 {BASE}/file/browser-test.txt 2>/dev/null"
    )
    assert "editor" in editor_dom, "Editor page missing editor element"
    assert "browser-test.txt" in editor_dom, "Editor page missing filename"

    # ── Chromium renders the editor by hash ───────────────────────────────
    edit_dom = server.succeed(
        f"chromium --headless --disable-gpu --no-sandbox --dump-dom "
        f"--timeout=15000 {BASE}/edit/{file_hash} 2>/dev/null"
    )
    assert "editor" in edit_dom, "Edit-by-hash page missing editor"

    # ── Verify JS-dependent UI elements rendered ──────────────────────────
    # The rename/copy buttons are in the static HTML, verify they're present
    assert "rename" in editor_dom.lower(), "Editor missing rename button"
    assert "copy" in editor_dom.lower(), "Editor missing copy button"

    # ── Verify theme is applied ───────────────────────────────────────────
    # Default theme is 'sneak', should be in data-theme attribute
    assert "sneak" in editor_dom, "Editor missing default theme"
  '';
}
