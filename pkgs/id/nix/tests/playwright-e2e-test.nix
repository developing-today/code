# NixOS VM Playwright E2E test — full browser coverage.
#
# Architecture: 4 VMs communicating over a virtual network
#   - chromium_server: id service on port 4173
#   - firefox_server:  id service on port 4174
#   - chromium_client: Playwright + Chromium tests against chromium_server
#   - firefox_client:  Playwright + Firefox tests against firefox_server
#
# This runs the FULL Playwright E2E suite (all spec files, both browsers)
# inside NixOS VMs where there are no sandbox restrictions. Chromium works
# properly here — unlike the nix build sandbox where it crashes.
#
# The existing `nixos-e2e` (chromium --dump-dom) is kept for fast DOM
# validation; this test provides full interactive browser coverage.
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

  # ── Server nodes: each runs an isolated id service instance ──────────────
  nodes.chromium_server =
    { ... }:
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
    };

  nodes.firefox_server =
    { ... }:
    {
      imports = [ ../id-module.nix ];

      services.id = {
        enable = true;
        package = idPackage;
        web = true;
        port = 4174;
        ephemeral = true;
        noRelay = true;
        noGossip = true;
        noMdns = true;
        openFirewall = true;
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

  globalTimeout = 600; # 10 minutes — browser startup + 146 tests

  testScript = ''
    E2E_RUNNER = "${e2eTestRunner}"
    BROWSERS = "${playwrightBrowsers}"

    # ── Wait for both servers ──────────────────────────────────────────────
    start_all()

    chromium_server.wait_for_unit("id.service")
    chromium_server.wait_for_open_port(4173)
    firefox_server.wait_for_unit("id.service")
    firefox_server.wait_for_open_port(4174)

    # ── Verify servers are reachable from client VMs ───────────────────────
    chromium_client.succeed("curl -sf http://chromium_server:4173/")
    firefox_client.succeed("curl -sf http://firefox_server:4174/")

    # ── Helper: copy test runner to writable dir and run Playwright ────────
    # The e2eTestRunner is a read-only nix store path; Playwright needs to
    # write test-results/ and playwright-report/ in the working directory.
    def run_playwright(client, project, base_url_var, base_url):
        client.succeed(
            f"cp -r {E2E_RUNNER} /tmp/e2e && "
            f"chmod -R u+w /tmp/e2e && "
            f"cd /tmp/e2e && "
            f"PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 "
            f"PLAYWRIGHT_BROWSERS_PATH={BROWSERS} "
            f"PLAYWRIGHT_VM_TEST=1 "
            f"{base_url_var}={base_url} "
            f"node node_modules/@playwright/test/cli.js test "
            f"--project={project} 2>&1"
        )

    # ── Run Chromium tests ─────────────────────────────────────────────────
    run_playwright(
        chromium_client, "chromium",
        "CHROMIUM_BASE_URL", "http://chromium_server:4173"
    )

    # ── Run Firefox tests ──────────────────────────────────────────────────
    run_playwright(
        firefox_client, "firefox",
        "FIREFOX_BASE_URL", "http://firefox_server:4174"
    )
  '';
}
