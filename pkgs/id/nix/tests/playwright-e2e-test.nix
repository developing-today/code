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
