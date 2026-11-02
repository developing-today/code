# NixOS VM integration test — runs the full cli_integration test suite
# including serve_tests (which need real networking unavailable in nix sandbox).
#
# Architecture: 1 VM with the pre-built integration test binary + id binary
#
# The test binary is compiled in a nix sandbox derivation (integrationTestRunner)
# with --no-run, then executed inside a VM where networking restrictions don't
# apply. The ID_BINARY env var tells tests where to find the id binary.
#
# This is the ONLY way to run serve_tests in nix — they spawn `id serve` as a
# subprocess and need to bind/listen on ports, which the nix build sandbox blocks.
#
# Usage:
#   pkgs.testers.runNixOSTest (import ./integration-test.nix {
#     inherit idPackage integrationTestRunner;
#   })
{ idPackage, integrationTestRunner }:
{
  name = "id-integration";

  nodes.server = _: {
    environment.systemPackages = [ ];
    virtualisation.memorySize = 2048;
    virtualisation.cores = 2;
  };

  # The complete cli_integration binary runs roughly 134 tests with two
  # workers, and individual child processes may wait up to three minutes.
  # World tests exercise multiple long-lived server processes and can run
  # serially behind the two-worker harness (see #190), so 20 minutes still
  # terminated the suite before it completed in CI. Keep enough headroom for
  # the full suite while preserving per-process deadlines in the test binary.
  globalTimeout = 3600; # 60 minutes for the full integration suite

  testScript = ''
    ID_BIN = "${idPackage}/bin/id"
    TEST_BIN = "${integrationTestRunner}/bin/cli_integration_test"
    EXAMPLES_DIR = "${integrationTestRunner}/share/id-test/examples"
    APPS_DIR = "${integrationTestRunner}/share/id-test/apps"

    start_all()

    # Verify both binaries are accessible
    server.succeed(f"test -x {ID_BIN}")
    server.succeed(f"test -x {TEST_BIN}")
    server.succeed(f"test -d {EXAMPLES_DIR}")
    server.succeed(f"test -d {APPS_DIR}")

    # Run the full integration test suite (including serve_tests).
    # ID_BINARY overrides the compile-time CARGO_BIN_EXE_id path so the
    # test binary finds the nix-built id binary.
    server.succeed(
        f"ID_BINARY={ID_BIN} "
        f"ID_TEST_EXAMPLES_DIR={EXAMPLES_DIR} "
        f"ID_TEST_APPS_DIR={APPS_DIR} "
        f"{TEST_BIN} "
         f"--test-threads=2 --nocapture --format=terse 2>&1"
    )
  '';
}
