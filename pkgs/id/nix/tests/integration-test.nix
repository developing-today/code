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

  globalTimeout = 300; # 5 minutes

  testScript = ''
    ID_BIN = "${idPackage}/bin/id"
    TEST_BIN = "${integrationTestRunner}/bin/cli_integration_test"

    start_all()

    # Verify both binaries are accessible
    server.succeed(f"test -x {ID_BIN}")
    server.succeed(f"test -x {TEST_BIN}")

    # Run the full integration test suite (including serve_tests).
    # ID_BINARY overrides the compile-time CARGO_BIN_EXE_id path so the
    # test binary finds the nix-built id binary.
    server.succeed(
        f"ID_BINARY={ID_BIN} "
        f"{TEST_BIN} "
        f"--test-threads=2 2>&1"
    )
  '';
}
