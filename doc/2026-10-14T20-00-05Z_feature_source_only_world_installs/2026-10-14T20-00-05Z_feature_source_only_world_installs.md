# Source-only world module installation

## Intent

### Request (verbatim)

> continue with any missing features. build out the platforms, capabilities, standard libraries across the feature set. ensure that most any ratatui shaped ssh app could be built using this tool set, and that any tui/ssh interface can usually be presented in web, either a direct translation or the app offers multiple presentation interfaces. ensure security sanbdoxing and other limits are customizable, in case an admin wants to Enable direct control. a server can also control access by what functions it allows in the roc interface and by only accepting untrusted code in source form, compiling the roc code itself and only using trusted sources. continue to fully implement all features one would expect for a general platform for data games communication etc.

This phase implements the source-only policy slice of that request; it does not declare the whole platform roadmap complete.

Add an operator-controlled mode for Roc worlds that accepts new guest programs only as Roc source compiled by this server's configured compiler. The normal Wasm path remains bounded and import-free. Raw Wasm/native artifact installation is a trusted-artifact feature and must be disabled entirely when source-only mode is enabled, including direct library calls and the streaming upload protocol.

This does not claim compiler supply-chain provenance: the executable is configured by the operator, but its version/digest is not currently recorded beside the active module. That remains a follow-up before compiler provenance can be claimed. Nor does source-only mean compilation is harmless; source size, compiler wall time and child process limits still apply.

## Interface and behavior

- Add `--world-source-only` (and `ID_WORLD_SOURCE_ONLY`) for `id serve --world`.
- Require a configured Roc compiler and reject `--world-module` at startup; those raw artifacts bypass the source path.
- Reject raw `install_wasm` and the WebSocket/Iroh streaming upload protocol before accepting bytes.
- Preserve `compile_and_install`: successful server-compiled output goes through the same hash validation, Wasm validator, module persistence, and atomic activation, with an internal marker distinguishing it from artifact upload.
- Do not implicitly enable native execution. If native compilation is requested, it still requires the separately opted-in OS-sandboxed native tier.
- Default remains backward compatible (`source-only = false`).

## Validation

Test both the CLI option constraints and `WorldService` behavior: raw artifacts are denied with/without admin auth; source compilation remains available when a compiler is configured; default mode still accepts an admin-authenticated Wasm module; and a source-only server cannot bootstrap from a raw `--world-module` file.

## Follow-up gaps

- Store compiler identity/version or digest with the installed module and expose it in module info.
- Add configurable compiler subprocess memory/CPU/file-size limits; current compiler limits include fixed time and output/source caps.
- Make resource profiles per-world and expose the implemented screen/payload limits as documented policy where appropriate.
- Keep native code isolated by Landlock/seccomp/rlimits; no unconfined host direct-control switch is introduced.
