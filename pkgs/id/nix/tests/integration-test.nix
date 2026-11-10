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
# Legibility: the suite used to run as one server.succeed() with --format=terse,
# so a hang produced only "timeout reached; test terminating..." with no test
# name or wait target (see #1372). It now runs as named groups, each under its
# own `timeout` deadline and with the pretty libtest format, so a hang names the
# group, the filters it was running, and (after 60 s) the individual tests that
# are still running, well inside globalTimeout. Group filters are verified to
# partition every test in the binary before any test runs, so coverage cannot
# silently shrink.
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
  # The testScript's own suite budget (3400 s) keeps every failure below this
  # backstop, so a hang is always reported by a named group instead of the
  # driver's global "timeout reached; test terminating...".
  globalTimeout = 3600; # 60 minutes for the full integration suite

  testScript = ''
    import shlex
    import time

    ID_BIN = "${idPackage}/bin/id"
    TEST_BIN = "${integrationTestRunner}/bin/cli_integration_test"
    EXAMPLES_DIR = "${integrationTestRunner}/share/id-test/examples"
    APPS_DIR = "${integrationTestRunner}/share/id-test/apps"

    # Named groups: (group name, libtest filter arguments, per-group deadline).
    # Filters are substring matches, but the coverage check below proves that
    # every test in the binary matches exactly one group. When a test module is
    # added, update this table (the check fails with the missing names).
    GROUPS = [
        (
            "fast_unit",
            [
                "cli_tests::",
                "put_get_tests::",
                "list_tests::",
                "find_search_tests::",
                "id_tests::",
                "peers_tests::",
                "error_handling_tests::",
                "show_view_tests::",
                "peek_tests::",
                "filter_flag_tests::",
                "show_peek_subcommand_help::",
            ],
            900,
        ),
        ("serve", ["serve_tests::"], 2200),
        ("tag", ["tag_tests::"], 400),
        (
            "harness_unit",
            [
                "test_output_within_kills_a_hung_process",
                "test_output_within_ends_when_a_grandchild_keeps_the_pipes",
                "test_unix_stop_keeps_its_gate_and_its_allow",
                "test_every_unsafe_block_is_allowed_on_its_function",
                "test_streaming_child_dies_with_its_guard_even_when_the_test_panics",
            ],
            300,
        ),
    ]

    # Total wall-clock budget for the groups, below globalTimeout (3600 s) so a
    # hang always fails here (with the group name) rather than at globalTimeout.
    # Leaves room for VM boot and the setup assertions above.
    SUITE_BUDGET_SECONDS = 3400

    start_all()

    # Verify both binaries are accessible
    server.succeed(f"test -x {ID_BIN}")
    server.succeed(f"test -x {TEST_BIN}")
    server.succeed(f"test -d {EXAMPLES_DIR}")
    server.succeed(f"test -d {APPS_DIR}")

    # The per-group deadline and the legibility of a killed child depend on
    # GNU timeout being present in the VM.
    server.succeed("command -v timeout")

    # Prove the group filters partition the suite before running anything.
    listed = server.succeed(f"{TEST_BIN} --list")
    all_tests = sorted(
        line.rsplit(": ", 1)[0]
        for line in listed.splitlines()
        if line.endswith(": test")
    )
    if not all_tests:
        raise AssertionError(
            "[id-integration] --list returned no tests; cannot verify group coverage"
        )

    covered = []
    for name, filters, _deadline in GROUPS:
        hits = [test for test in all_tests if any(f in test for f in filters)]
        if not hits:
            raise AssertionError(
                f"[id-integration] group '{name}' matches no tests; update GROUPS"
            )
        covered.append((name, hits))

    overlap = sorted({t for _name, hits in covered for t in hits if hits.count(t) > 1})
    missing = sorted(set(all_tests) - {t for _name, hits in covered for t in hits})
    if overlap or missing:
        raise AssertionError(
            "[id-integration] group filters must partition the suite exactly; "
            f"duplicated={overlap} uncovered={missing}"
        )

    print(
        "[id-integration] "
        f"{len(all_tests)} tests in {len(GROUPS)} named groups: "
        + ", ".join(f"{name}={len(hits)}" for name, hits in covered),
        flush=True,
    )

    env = (
        f"ID_BINARY={ID_BIN} "
        f"ID_TEST_EXAMPLES_DIR={EXAMPLES_DIR} "
        f"ID_TEST_APPS_DIR={APPS_DIR}"
    )

    suite_started = time.monotonic()
    failed_groups = []
    for name, filters, cap in GROUPS:
        elapsed = int(time.monotonic() - suite_started)
        remaining = SUITE_BUDGET_SECONDS - elapsed
        if remaining < 60:
            raise AssertionError(
                f"[id-integration] suite budget of {SUITE_BUDGET_SECONDS}s "
                f"exhausted after {elapsed}s, before group '{name}' "
                f"(filters: {' '.join(filters)})"
            )

        deadline = min(cap, remaining)
        filter_args = " ".join(shlex.quote(f) for f in filters)
        print(
            f"[id-integration] waiting on group '{name}' "
            f"(filters: {filter_args}); per-group deadline {deadline}s, "
            f"{remaining}s of suite budget left",
            flush=True,
        )

        group_started = time.monotonic()
        try:
            server.succeed(
                f"{env} timeout --signal=TERM --kill-after=15 {deadline} "
                f"{TEST_BIN} {filter_args} "
                f"--test-threads=2 --nocapture --format=pretty 2>&1"
            )
        except Exception:
            # The driver has already logged the captured output, including the
            # last started test and libtest's "has been running" warnings.
            # Record the group, keep going so every group still runs, and fail
            # with a summary naming all failed groups at the end.
            failed_groups.append(name)
            print(
                f"[id-integration] group '{name}' failed or exceeded its "
                f"{deadline}s deadline; filters: {filter_args}; continuing",
                flush=True,
            )
        else:
            print(
                f"[id-integration] group '{name}' finished in "
                f"{time.monotonic() - group_started:.1f}s",
                flush=True,
            )

    if failed_groups:
        raise AssertionError(
            "[id-integration] failed groups: " + ", ".join(failed_groups)
        )
        print(
            f"[id-integration] group '{name}' finished in "
            f"{time.monotonic() - group_started:.1f}s",
            flush=True,
        )
  '';
}
