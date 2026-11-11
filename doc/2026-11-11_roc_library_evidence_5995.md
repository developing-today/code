# Roc shared-library outputs and host loading: evidence for #5995

Status: **complete**. Evidence for [#5995](https://github.com/developing-today/code/issues/5995) and the dynamic user functions design [#5964](https://github.com/developing-today/code/issues/5964). All experiments ran 2026-11-10/11 on the fleet laptop (Linux x86-64, NixOS) against the repository at the current main. This document replaces the earlier partial checkpoint of the same name; the one interim conclusion that survives unchanged is the design constraint in the last section, now backed by artifacts instead of recollection.

## The two compiler lines

| Line | Compiler | Source |
| --- | --- | --- |
| Pinned / tracked (new compiler) | `nightly-2026-10-04-130536d` (pin), `nightly-2026-10-06-c34079d` (installed) | `pkgs/id/nix/roc-version`, `pkgs/id/nix/roc.nix`; system `roc` on the laptop |
| Legacy (old compiler) | `alpha4-rolling`, internally `roc_nightly-linux_x86_64-2025-09-09-d73ea109`, "roc nightly pre-release, built from commit d73ea109" | `roc-lang/roc` GitHub release `alpha4-rolling`, asset `roc-linux_x86_64-alpha4-rolling.tar.gz`, sha256 `96e8be05e6f7176433ada74532ff36a62b8dc44c5247a82cdf919f2dadc5178b` |

The checked-in `pkgs/roc` build scripts (`pkgs/roc/lib/build.sh`, `pkgs/roc/applications/hello/rust-basic-cli/Platform/Lib/build.sh`) invoke `roc build --lib` and `roc preprocess-host`; those are legacy-compiler commands, as the next section shows.

## New compiler: no library output at all

- `roc build --help` on nightly-2026-10-06 lists no `--lib` flag; `roc build` produces a binary (targets include Wasm). The pinned 2026-10-04 nightly's help likewise has no `--lib` (checked in the earlier checkpoint via the nix-built pin at `/nix/store/8z9ryv4w9ci368qbg62fiyfv7zwjh92d-roc-nightly-2026-10-04-130536d`).
- The full subcommand list has no `preprocess-host`.
- Direct attempt, verbatim result: `roc build main.roc --lib --output libapp.so` → `Error: roc build received an unexpected argument: --lib`.

So on the compiler line the repository actually pins and tracks, there is no shared-library artifact to load, swap, or design around. Any DLL-based design would first require adopting the legacy compiler line.

## Legacy compiler: `roc build --lib` works, and here is the artifact

The legacy compiler's `roc build --help` lists `--lib` ("Build a C library instead of an executable"), `--linker <surgical|legacy>`, and a `preprocess-host` subcommand ("Runs the surgical linker preprocessor to generate `.rh` and `.rm` files").

Environment note (a real adoption cost): the legacy binary needs `libz`/`libzstd`/`libtinfo`/`libstdc++` at runtime and hard-fails its link step unless FHS libc development files (`crti.o`, `crtn.o`, `Scrt1.o`, `libgcc_s.so.1`) exist under `/usr/lib/x86_64-linux-gnu` — it searches only `/usr/lib/x86_64-linux-gnu`, `/lib/x86_64-linux-gnu`, `/usr/lib64`, `/usr/lib` and honors no `LIBRARY_PATH` override. On this NixOS laptop the builds below ran inside a user namespace with those files bind-provided from the nix store glibc. The repo's own `build.sh` scripts also no longer run as-is: their getopt loop dies with `line 22: 1: unbound variable` under `set -u`, so the commands below were run directly.

Build (legacy roc, in `pkgs/roc/applications/hello/rust-basic-cli`):

```sh
roc build --lib main.roc --output libapp.so
# 0 errors and 0 warnings … successfully building: libapp.so
```

Artifact inspection (`libapp.so`, the rust-basic-cli hello app):

- `file`: ELF 64-bit LSB shared object, x86-64, dynamically linked, with debug_info, not stripped; 99,672 bytes; sha256 `d4841f5d8d64e5e558fb1efcaffa6c3267c6d42bdf17cbb7876abf0b778c7224`.
- Dynamic `NEEDED`: only `libc.so.6`, `libm.so.6`, `libpthread.so.0`, `libdl.so.2`, `librt.so.1`, `libutil.so.1`, `libgcc_s.so.1`.
- Exported entry surface is exactly the platform ABI triple: `roc__main_for_host_1_exposed`, `roc__main_for_host_1_exposed_generic`, `roc__main_for_host_1_exposed_size`.
- Undefined symbols the **host must provide**: `roc_alloc`, `roc_dealloc`, `roc_realloc`, `roc_panic`, and the platform effects `roc_fx_log`, `roc_fx_stdout_line`. The library is not self-contained: it binds against host-provided allocator and effect functions at load time.

## How the intended host flow loads the library: startup dynamic linking

Using the Go host (the Rust host's `build.roc` built its stub `platform/libapp.so` but its cargo step failed in this environment fetching the `roc_std` git dependency — recorded as an environment failure, not a compiler verdict):

```sh
roc build --lib main.roc --output libapp.so        # applications/hello/go-basic-cli, 43,048 bytes
CGO_LDFLAGS="-L<app dir> -Wl,-rpath,$ORIGIN/../applications/hello/go-basic-cli" \
  go build -buildmode=pie -o dynhost .             # platforms/go-basic-cli, 2,621,432 bytes
roc preprocess-host ./dynhost ./main.roc <app dir>/libapp.so   # succeeded
```

- `ldd dynhost` resolves `libapp.so` as a `NEEDED` dependency through the `$ORIGIN`-relative rpath — i.e. the operating system's dynamic linker loads the app library **at process startup**, as part of ordinary dynamic linking. `preprocess-host` prepares metadata (`.rh`/`.rm`) for this; it does not produce a runtime loader.
- `./dynhost` runs the app ("Roc loves Go", …). With `libapp.so` renamed away, startup fails immediately: `./dynhost: error while loading shared libraries: libapp.so: cannot open shared object file`.
- Replacement test: the app's `Lib/Hello.roc` string was changed, `libapp.so` rebuilt, and the **same unmodified `dynhost`** printed the new string ("SWAPPED Roc loves Go"); restoring the original `.so` restored the original output. So the toolchain's replacement granularity is: **replace the file, restart the process**. A running process keeps the code it started with; there is no in-process swap anywhere in this flow.

## Runtime loading (dlopen) is mechanically possible but is not the toolchain's mechanism

A minimal C host exporting the six host-ABI symbols above (`-rdynamic`) was used to probe raw runtime loading of the rust-basic-cli `libapp.so`:

- `dlopen(path, RTLD_NOW | RTLD_LOCAL)` succeeds; `dlsym` resolves `roc__main_for_host_1_exposed` and `roc__main_for_host_1_exposed_size`; `dlclose` followed by a second `dlopen` of the same path also succeeds.
- Negative control: the same `dlopen(RTLD_NOW)` from a host that does **not** export the ABI symbols fails: `libapp.so: undefined symbol: roc_dealloc`.

So an ELF-level runtime load/reload is possible **if the host itself implements it** — supplying the allocator/effect ABI, managing handle lifetime, and deciding what happens to in-flight calls and Roc heap state across a `dlclose` (nothing in the artifact or toolchain manages any of that; `roc_alloc`-allocated state crossing a reload is the host's problem). The compiler provides no flag, subcommand, or runtime support for loading, unloading, or swapping a library inside a running host on either compiler line.

## Conclusion for #5964

1. **Pinned/tracked compiler: no DLL path exists.** No `--lib`, no `preprocess-host`, binary (or Wasm) outputs only. Designing per-user functions around native Roc DLLs would mean switching the world to the legacy compiler line — a pin change in the opposite direction of the current one.
2. **Legacy compiler: the DLL path is startup linking, not hot-swap.** A Roc app compiles to a host-ABI-bound `.so`; the supported flow links it into the host's dynamic dependencies and loads it at process start. Code replacement = new file + new process.
3. **Therefore: do not design around native Roc DLL hot-swap.** The swap semantics #5964 needs (tick-boundary swap, in-flight completion, state handoff) must live at the host, Wasm, or process layer — e.g. a host that dlopens per-user libraries itself with an explicit ABI/version contract, whole-process replacement per publish, or the Wasm target the new compiler already builds. This confirms, with artifacts, the interim constraint recorded in the earlier checkpoint.

Reproduction: all commands above are runnable from `pkgs/roc` with the legacy compiler on `PATH` and the FHS libc files present; the dlopen probe and namespace build scripts used for this evidence are small and described inline (host stubs: the six `roc_*` symbols listed in the artifact section).
