# NixOS VM integration test for the `id` web server.
#
# Tests HTTP endpoints, file creation via API, and basic web UI rendering.
# Runs inside a NixOS VM with full loopback network — no sandbox restrictions.
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
        enable = true;
        package = idPackage;
        web = true;
        port = 3000;
        ephemeral = true;
        noRelay = true;
        noGossip = true;
        noMdns = true;
        openFirewall = true;
      };

      environment.systemPackages = [ pkgs.curl ];
    };

  globalTimeout = 120;

  testScript = ''
    import json

    PORT = 3000
    BASE = f"http://localhost:{PORT}"

    start_all()

    # ── Boot & service readiness ──────────────────────────────────────────
    server.wait_for_unit("id.service")
    server.wait_for_open_port(PORT)

    # ── Home page renders ─────────────────────────────────────────────────
    html = server.succeed(f"curl -sf {BASE}/")
    assert "Files" in html, f"Home page missing 'Files' heading: {html[:200]}"

    # ── Static assets served ──────────────────────────────────────────────
    server.succeed(f"curl -sf -o /dev/null {BASE}/assets/manifest.json")

    # ── Create a file via API ─────────────────────────────────────────────
    create_resp = server.succeed(
        f"curl -sf -X POST {BASE}/api/new "
        f"-H 'Content-Type: application/json' "
        f"-d '{{\"name\": \"hello.txt\"}}'"
    )
    resp = json.loads(create_resp)
    assert "hash" in resp, f"Create response missing 'hash': {create_resp}"
    assert resp.get("name") == "hello.txt", f"Unexpected name: {resp}"
    file_hash = resp["hash"]

    # ── File appears in list ──────────────────────────────────────────────
    list_html = server.succeed(f"curl -sf {BASE}/")
    assert "hello.txt" in list_html, "Created file not in file list"

    # ── File accessible by name ───────────────────────────────────────────
    file_html = server.succeed(f"curl -sf {BASE}/file/hello.txt")
    assert "hello.txt" in file_html, "File page missing filename"

    # ── File accessible by hash ───────────────────────────────────────────
    edit_html = server.succeed(f"curl -sf {BASE}/edit/{file_hash}")
    assert "hello.txt" in edit_html, "Edit page missing filename"

    # ── Save content via API ──────────────────────────────────────────────
    save_resp = server.succeed(
        f"curl -sf -X POST {BASE}/api/save "
        f"-H 'Content-Type: application/json' "
        f"-d '{{\"name\": \"hello.txt\", \"content\": \"Hello, NixOS!\"}}'"
    )
    save = json.loads(save_resp)
    assert save.get("success") == True or "hash" in save, f"Save failed: {save_resp}"

    # ── Rename via API ────────────────────────────────────────────────────
    rename_resp = server.succeed(
        f"curl -sf -X POST {BASE}/api/rename "
        f"-H 'Content-Type: application/json' "
        f"-d '{{\"name\": \"hello.txt\", \"new_name\": \"renamed.txt\", \"archive\": false}}'"
    )
    rename = json.loads(rename_resp)
    assert rename.get("name") == "renamed.txt", f"Rename failed: {rename_resp}"

    # ── Renamed file accessible ───────────────────────────────────────────
    server.succeed(f"curl -sf {BASE}/file/renamed.txt")

    # ── Copy via API ──────────────────────────────────────────────────────
    copy_resp = server.succeed(
        f"curl -sf -X POST {BASE}/api/copy "
        f"-H 'Content-Type: application/json' "
        f"-d '{{\"name\": \"renamed.txt\", \"new_name\": \"copy.txt\"}}'"
    )
    copy = json.loads(copy_resp)
    assert copy.get("name") == "copy.txt", f"Copy failed: {copy_resp}"

    # ── Both files exist ──────────────────────────────────────────────────
    server.succeed(f"curl -sf {BASE}/file/renamed.txt")
    server.succeed(f"curl -sf {BASE}/file/copy.txt")

    # ── Delete via API ────────────────────────────────────────────────────
    delete_resp = server.succeed(
        f"curl -sf -X POST {BASE}/api/delete "
        f"-H 'Content-Type: application/json' "
        f"-d '{{\"name\": \"copy.txt\"}}'"
    )

    # ── Download endpoint ─────────────────────────────────────────────────
    server.succeed(
        f"curl -sf -o /dev/null {BASE}/api/download/renamed.txt"
    )

    # ── Health: server still running after all operations ─────────────────
    server.succeed(f"curl -sf {BASE}/")
  '';
}
