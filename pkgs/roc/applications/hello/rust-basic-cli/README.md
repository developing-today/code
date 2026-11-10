# hello — rust-basic-cli

A Roc `hello` application on a vendored Rust basic-cli platform
(`Platform/main.roc`, platform name `rust-basic-cli`).

## The host archive is generated, not tracked

Running this example with the legacy linker needs a prebuilt host
archive at `Platform/linux-x64.a` (on Linux x86-64; the equivalent
files for other targets are named `linux-arm64.a`, `macos-x64.a`,
`macos-arm64.a`, and so on). That archive is a compiled build output,
so it is deliberately not tracked in git — the root `.gitignore`
rule `**/*.a` covers it. A fresh clone therefore does not contain it,
and it must be regenerated before the first run:

1. Build the platform in the template this platform was copied from:

   ```sh
   cd ../../../platforms/rust-basic-cli-template
   roc build.roc
   ```

   That runs `cargo build --release` for the host crates and copies
   the resulting `libhost.a` to `platform/linux-x64.a` for the native
   target (see `build.roc`'s `copy_host_lib`; `bundle.sh` does the
   same copy for all targets at once).

2. Copy the archive for your target into this app's platform:

   ```sh
   cp platform/linux-x64.a \
     ../../applications/hello/rust-basic-cli/Platform/linux-x64.a
   ```

3. Run the example from `pkgs/roc`:

   ```sh
   LINKER=legacy ./hello rust-basic-cli
   ```

Rebuilding the host overwrites the archive in place; because it is
ignored, rebuilds no longer dirty the working tree.
