# Android `arm64-v8a` target for the `id-client` facade (slice 1, #7938/#13403)

## What lives here

- `check-android.sh`: the slice-1 Android gate. Runs the
  `android_dependency_boundary` check, then `cargo check -p id-client
--target aarch64-linux-android` when the target std is installed.
- This README: the build path contract for the later Kotlin shell.

## Prerequisites

- Rust target std: `rustup target add aarch64-linux-android`
  (toolchain file `pkgs/id/rust-toolchain.toml`, currently 1.99.1).
- Android NDK r27+ with `aarch64-linux-android35-clang` on `PATH`, or
  set `ANDROID_NDK_HOME`. `cargo check` needs only the target std;
  full `cargo build` / APK linkage needs the NDK linker.

## Commands (run from `pkgs/id/`)

```sh
./android/check-android.sh
```

The script is pass/fail: boundary test first, then the Android target
resolve. Its output names the exact commands and results for the PR
record. No world hosting, drive, recovery, provider, push, or presence
surface is built here — client facade only.
