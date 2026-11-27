# Board-game runtime artifact prerequisite (#15112)

The games already have shared wasm artifacts (#15632); #16123 adds cross-run reuse. The laptop still needs a runnable `id` with `sandbox,world`. A copied Ubuntu binary is not a reliable NixOS executable because its interpreter and shared libraries may be absent.

Use the existing `id-lib` flake package (feature source: `pkgs/id/composition/variants.toml`) and export its complete Nix runtime closure, following `.github/scripts/id-shell-release.sh`. A manual-only workflow builds once on Actions and publishes the closure, exact store path, source SHA and checksums. Joining machines with Nix import it and invoke the exact store path; no profile changes, local checkout, or compiler is needed. This intentionally does not change the wasm artifact or the live-game protocol.

Verified when: the focused packaging test proves strict argument parsing before subprocesses/writes, nonempty closure export, CLI world/serve probes, checksums, provenance and error propagation. A successful manual workflow must additionally prove the real package builds and uploads. An artifact-only laptop run then imports this closure and executes `id world join --help`; that operational measurement is not implied by contract tests.

Next: import this runtime plus the existing wasm artifact on the laptop, orchestrate Actions/Blacksmith/laptop over the id network and record the completed game and run/artifact IDs on #14862. No three-machine success or issue completion is claimed by this prerequisite.
