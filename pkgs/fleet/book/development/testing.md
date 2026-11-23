# Testing

Fleet's testing doctrine comes from the repository's standing rules: property-based tests where code has meaningful invariants, model-based tests where a spec or model exists, and **every automated test names its oracle** — in practice, test names and comments carry the issue number and the invariant they check (`Oracle (#7598): …`, `#12777 oracle: the unblock marker is audit-only…`). `proptest 1.12` is a dev-dependency of the binary crate (for the `gh` read classifier) and of `fleet-queue` (for the client parser invariants).

Run the whole suite from `pkgs/fleet/`:

```sh
cargo test --workspace
```

## Inline, oracle-named tests per crate

Tests are overwhelmingly inline `#[cfg(test)] mod tests` in the file they test. The shape per crate:

| Crate                                                  | Where the tests live                                                                                                                                                                                                                                                                                                                                                                      |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fleet` (bin)                                          | Inline tests in `src/main.rs` (serve-list parsing, proxy-ssh argv building, help-flag positions, broker-role argument rejection) plus integration targets in `tests/`: `invocation_ledger.rs`, `proxy_ssh.rs`, `search_serve.rs`.                                                                                                                                                         |
| `fleet-registry`                                       | In-file invariant tests (`capability_ids_are_unique_and_stable`, `variant_profiles_nest`, `every_domain_primary_is_in_its_surface`, `aliases_route_to_real_domains`, `domain_aliases_match_the_argv0_map`) plus the integration target `tests/pmo_operator_docs.rs` — the DOC-01 machine check described in [Keeping this book current](keeping-this-book-current.md).                    |
| `fleet-queue`                                          | A very large inline test module in `src/client.rs` (parser invariants, the usage↔parser parity test below, timeline rendering), marker-fold oracles in `src/fallback.rs`, tests in the `src/client/` command modules, and integration targets gated by feature (next section). Two Python SQL regressions ship as test data: `implementation_repo_sql.py` and `quarantine_atomic_sql.py`. |
| `fleet-config`                                         | A large inline suite (around fifty tests) plus proptest.                                                                                                                                                                                                                                                                                                                                  |
| `fleet-secrets`                                        | A large inline suite (around thirty-five tests).                                                                                                                                                                                                                                                                                                                                          |
| `fleet-search`                                         | `tests/serve_contract.rs` for the serve-role contract. The server is a parity-guarded copy of the atlas-search service, also guarded by the `fleet-search-parity-guard` workflow.                                                                                                                                                                                                         |
| `fleet-agent`                                          | Inline `src/tests.rs`; proptest regression seeds are checked in at `proptest-regressions/tests.txt`.                                                                                                                                                                                                                                                                                      |
| `fleet-dispatch`, `fleet-laptop-agent`, `fleet-bridge` | Inline tests in `src/lib.rs` / `src/tests.rs` and module tests.                                                                                                                                                                                                                                                                                                                           |

## Feature-gated integration tests: `[[test]] required-features`

Because the workspace is feature-split (see [Register a domain or capability](register-a-domain.md)), a test target declares the features it needs, and Cargo only builds it when that surface is compiled. In `fleet-queue`'s `Cargo.toml`:

- `tests/broker_bin_args.rs` — `required-features = ["queue-broker", "secrets-broker"]`
- `tests/chunked_list.rs` — `required-features = ["client"]`
- `tests/person_wait_cli.rs` — `required-features = ["client"]`

The consequence to remember: the fallback module sits behind the `client` feature (`#[cfg(feature = "client")] pub mod fallback;` in `crates/fleet-queue/src/lib.rs`), so **a broker-only test run does not compile the fallback tests at all**. The fallback oracles run only under a client-carrying feature set. The Nix `fleet-brokers` test flags and the full-variant check phase both carry `client`; a local `cargo test --features queue-broker` run silently skips them. When you change fallback behaviour, run a client-carrying test set explicitly.

## `pkgs/fleet/tests/` — the one Python test

`pkgs/fleet/tests/` contains exactly one file, and it is not a Cargo integration test: `test_tailnet_proxy_env.py`, a Python test of the tailnet proxy environment contract for the Python-side tooling that shares the tree.

Other test-bearing corners outside the crates: `pkgs/fleet/archer-slack/` Python tests, `pkgs/fleet/hosted-worker/test/` TypeScript tests for the hosted Worker edition (including `stamp-sig.test.ts`, the golden-vector conformance surface the Rust `gh` domain's Ed25519 signing must reproduce byte-exact), `pkgs/fleet/placement/evaluate_test.go` for the Go placement proof of concept, and `pkgs/fleet/migrations/*_test.sql`.

## `smoke.sh` — the hermetic post-build suite

`pkgs/fleet/smoke.sh` is the per-variant, post-build smoke suite. It is hermetic by contract: no network, no installs, no state outside a `mktemp` directory, and it exits non-zero if any check fails.

1. Build the four outputs (see [Building fleet](building.md)).
2. Invoke it with the four Nix package output paths; each argument's `bin/fleet` is the artefact under test:

   ```sh
   ./pkgs/fleet/smoke.sh ./result ./result-1 ./result-2 ./result-3
   ```

What it covers:

- **Variants** — for each of full, serve, connect, and static: `--help` renders the full registry with unavailable domains marked, and `version` lists exactly that variant's capability set.
- **Stubs** — a missing domain stubs with exit 69 and the right wording, reached three ways: `fleet <domain>`, an argv0 alias symlink, and a hyphenated variant name (which narrows). A config-disabled domain stubs at 69 with the config wording.
- **Connect hygiene** — both connect flavours run under `env -i` and are within the D7 size gate (≤ 2 MiB).
- **Search serve role** — reaches the real role rather than a stub; hermetically it stops at config load (exit 2) before any token is needed.
- **Proxy domain end to end** — broker auto-start, round-trips, shim materialisation and stale-shim removal on a services-map change, init no-op, location-aware init cases (an unknown location reports and exits 69 with no broker spawned), legacy proto-0 interop, idle exit, pool default rendering, a 32-concurrent burst, and the identity stamp.
- **Queue domain** — the secrets-broker role binds and `/health` answers (in the hermetic world, 503 with `serving=false` is the _pass_ shape); the queue-broker `/healthz` answers; client-to-broker wiring (a credential failure surfaces as exit 3 with broker wording); the fallback switch (one JSON object, exit 6) and the operator escape hatch underneath it; client `--help` / `--version` through the argv0 route.
- **Dispatch domain** — the thin client against a stub pm-dispatch and a stub `bws` on `PATH`: the token is resolved by name, presented as `Authorization: Bearer`, the service JSON is printed verbatim, and a deliberate 409 is passed through.
- **Secrets domain** — the broker-first chain against a stub session broker speaking the framed protocol, with the `bws` fallback behind it.

## CI: main only

Repository policy: CI runs only on `main`. There are no pull-request runs, required status checks are disabled, and formatting is not a merge gate (it is auto-fixed on `main`). The fleet-relevant workflows:

| Workflow                        | Trigger                                                                                                                                           | What it runs                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fleet-rust-tests.yml`          | Nightly cron (`17 7 * * *`) plus `workflow_dispatch` only                                                                                         | A contract check (`check-fleet-rust-tests.sh`, plus that script's own test), installs toolchain 1.99.1 via rustup (the `rust-toolchain.toml` pin), then `cargo +1.99.1 test --manifest-path pkgs/fleet/Cargo.toml --workspace --all-targets --locked`. Its header records why it exists: before it, the workspace's tests ran in no CI venue at all, and the baseline run found a stale dep-wiring oracle and a parallel-spawn flake in `fleet-queue`.                                                                                                                          |
| `fleet-smoke.yml`               | Push to `main` with paths under `pkgs/fleet/**`, the atlas-search manifests/crates, the workflow, and its check scripts; plus `workflow_dispatch` | Contract checks, the **fleet `Cargo.lock` versus atlas manifests check** (`check-fleet-lock.sh` — it fails by name instead of deep inside a derivation when the lockfile has drifted from the atlas-search path dependencies), Nix-builds the four outputs `.#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static`, runs `pkgs/fleet/smoke.sh` on them, and greps the log for `^smoke: [1-9][0-9]* passed, 0 failed$` so a silently skipped suite cannot report green. Timeout 150 minutes (a cold cache compiles ONNX Runtime from source; warm is roughly 40 minutes). |
| `fleet-search-parity-guard.yml` | Per its own triggers                                                                                                                              | Guards the "verbatim copy" claim of `fleet-search`'s `server.rs` against the atlas-search service.                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |

Each fleet workflow carries a contract step — a shell check script _and that script's own test_ run before the real work — so a workflow edited out of sync with the test-suite registry fails by name.

> Discrepancy note (workflow text): `fleet-rust-tests.yml`'s concurrency comment speaks of push-to-main runs, but the workflow has no push trigger at all — nightly plus dispatch only, by a deliberate ruling. The header's nightly note is the true statement; the comment is leftover boilerplate.

## The usage↔parser parity test — and what it does not check

Queue usage text is hand-maintained in two strings per verb, both in `crates/fleet-queue/src/client/support.rs`: the one-line synopsis in `TOP_USAGE` and the long text returned by `usage(verb)`. The parser is hand-rolled (`parse_verb_args` / `parse_args_inner` in the same file). The machine check that keeps them honest is `every_verb_usage_matches_its_parser` in the `client.rs` test module: it asserts that every verb's usage names _exactly_ the flags its parser accepts. Companion tests are `usage_covers_the_acceptance_verbs` and `usage_never_prints_nothing` (an unknown verb falls back to `TOP_USAGE`).

**What it does not cross-check: `TOP_USAGE`.** The parity test checks each verb's own `usage()` text against its parser; it does not check the `TOP_USAGE` synopsis lines against either. That drift is real today: for `release`, `heartbeat`, `checkpoint`, `fail`, and `complete`, the per-verb `usage()` text carries `[--lane <id>]` while the `TOP_USAGE` synopsis omits `--lane`. The parser is the authority on which spelling is right for each verb. Until a cross-check exists, anyone editing a verb's flags must update both strings by hand — the procedure in [Add a flag](add-a-flag.md) says exactly that.
