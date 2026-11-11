# Roc library embedding: partial evidence for #5995

Status: **incomplete**. This is a checkpoint for [#5995](https://github.com/developing-today/code/issues/5995) and the dynamic user functions design [#5964](https://github.com/developing-today/code/issues/5964), not proof that either compiler line supports or forbids runtime loading. No shared library was successfully built or loaded in this investigation.

## Pinned new compiler

`pkgs/id/nix/roc-version` pins `nightly-2026-10-04-130536d`; `pkgs/id/nix/roc.nix` describes it as the official new-compiler nightly and installs its `roc` executable. On Linux x86-64, the following built the package and `bin/roc --version` reported that exact nightly:

```sh
nix build --no-link --print-out-paths --impure --expr \
  'let p = import (builtins.getFlake (toString ./.)).inputs.nixpkgs {}; in p.callPackage ./pkgs/id/nix/roc.nix {}'
```

The output was `/nix/store/8z9ryv4w9ci368qbg62fiyfv7zwjh92d-roc-nightly-2026-10-04-130536d`. Its `roc build --help` did not list `--lib`. Trying `roc build --lib pkgs/roc/platforms/rust-minimal-cli/platform/libapp.roc --output <temporary-path>` exited 1 with an unexpected input-path argument. This demonstrates that this invocation is not accepted; it does **not** establish what other library output mechanisms might exist.

## Older app-library path and outstanding experiment

The checked-in `pkgs/roc/lib/build.sh` invokes `roc build $linker --lib` to produce `libapp.so`, then builds a platform host and invokes `roc preprocess-host`. The related `pkgs/roc/applications/hello/rust-basic-cli/Platform/Lib/build.sh` includes a Go host path using `CGO_LDFLAGS` with `-L` and an rpath. These are build scripts, not inspected outputs or proof of runtime hot-swap. They also do not establish compatibility with the pinned new compiler.

A legacy `alpha4-rolling` release was identified as a candidate, not as a repository pin or a verified representative revision. Downloading its Linux x86-64 archive timed out after approximately 22 of 37 MiB, so no legacy artifact or loading experiment was performed. The separately available `nightly-2026-10-06-c34079d` is also a new compiler, not a substitute for the legacy line.

To finish #5995: identify the exact tracked/legacy revision intended for #5964; obtain that compiler; build a representative library; record version, source, commands and `file`/dynamic-symbol/dependency inspection; run a host that links or explicitly loads and calls it, distinguishing startup linking from runtime loading and replacement. Repeat for any actual library mechanism found in the pinned compiler. Link the finished evidence from #5964.

**Interim design constraint:** do not design around native Roc DLL hot-swap on the basis of these scripts or recollection. Evaluate host-, Wasm-, or process-layer replacement independently; this checkpoint does not prove native hot-swap impossible.
