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
        file_html = server.succeed(f"curl -sf {BASE}/edit/hello.txt")
        assert "hello.txt" in file_html, f"[port {port}] File page missing filename"

        # ── File accessible by hash ───────────────────────────────────────
        edit_html = server.succeed(f"curl -sfL {BASE}/hash/{file_hash}")
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
        server.succeed(f"curl -sf {BASE}/edit/renamed.txt")

        # ── Copy via API ──────────────────────────────────────────────────
        copy_resp = server.succeed(
            f"curl -sf -X POST {BASE}/api/copy "
            f"-H 'Content-Type: application/json' "
            f"-d '{{\"name\": \"renamed.txt\", \"new_name\": \"copy.txt\"}}'"
        )
        copy = json.loads(copy_resp)
        assert copy.get("name") == "copy.txt", f"[port {port}] Copy failed: {copy_resp}"

        # ── Both files exist ──────────────────────────────────────────────
        server.succeed(f"curl -sf {BASE}/edit/renamed.txt")
        server.succeed(f"curl -sf {BASE}/edit/copy.txt")

        # ── Delete via API ────────────────────────────────────────────────
        server.succeed(
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
