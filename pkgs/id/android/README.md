# Android PoC shell and `id-client` target (#1831 / #7938)

## What lives here

- `check-android.sh`: the slice-1 Android gate. Runs the
  `android_dependency_boundary` check, then `cargo check -p id-client
--target aarch64-linux-android` when the target std is installed.
- `app/`: a minimal launcher APK shell. It intentionally reports the
  disconnected state; it does not yet connect, relay input, or host an
  id node. It requests no permissions, including no VPN permission.
- `check-android-shell.sh`: SDK-free structural smoke check for launcher
  wiring, explicit disconnected UI, and the absence of permissions.
- This README: the build path contract for the Kotlin shell.

## Android app shell

The project uses Gradle Kotlin DSL, Android Gradle Plugin 8.7.3, Kotlin
2.0.21, compile/target SDK 35, and min SDK 26. With Gradle and the Android
SDK installed, build an APK from this directory with
`gradle :app:assembleDebug`; the artifact is
`app/build/outputs/apk/debug/app-debug.apk`. No Gradle wrapper is checked
in yet. `bash check-android-shell.sh` checks the project wiring without
requiring either Gradle or an emulator.

This is an executable shell checkpoint only. iroh transport, binding the
Rust `GestureTranslator`, cell-coordinate mapping, ID record storage/serving,
and live-app tap/latency demonstration remain follow-on work; do not treat
the presence of this APK shell as satisfying #1831's relay acceptance.

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
