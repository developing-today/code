# Android remote-drive PoC (#1831)

## What this slice does

- `app/`: a foreground Android shell that accepts an Iroh host address, an
  optional world name, and a join capability; it joins the existing
  `/id-world/1` session and displays validated host views.
- `pkgs/id/crates/id-android-remote`: a small Rust JNI bridge. It reuses the
  shell contract and `WorldClient`, keeps the transport endpoint in a Rust
  runtime thread, exposes explicit connection errors, and never starts a host
  or listener.
- The capability is used for the join only, is not logged or persisted, and
  the UI field does not save its contents. The APK requests only Android's
  normal `INTERNET` permission; it does not request `VpnService`.
- `check-android-shell.sh`: SDK-free structural check for manifest permission
  shape, JNI wiring, and explicit disconnected/error states.

Tap/drag/scroll relay, cell mapping, ID-record storage/serving, and live-screen
click-through remain later work on #1831. The presence of a connected view does
not claim that input relay works.

## Build

The Gradle `preBuild` task cross-compiles the Rust library for emulator and
device ABIs into `app/src/main/jniLibs`. Install the Rust toolchain from
`pkgs/id/rust-toolchain.toml`, `cargo-ndk` 4.1.2, Android SDK 35, and an Android
NDK r27+; then run from this directory:

```sh
gradle :app:assembleDebug
bash check-android-shell.sh
```

Fleet CI uses the `ghcr.io/cirruslabs/android-sdk:35-ndk` image and installs
the pinned Rust toolchain plus `cargo-ndk` before building. The debug APK is
`app/build/outputs/apk/debug/app-debug.apk`.

## Runtime inputs

- **Host address:** an Iroh endpoint ID, or a serialized Iroh `EndpointAddr`
  when direct transport addresses are needed.
- **World:** optional world name; blank uses the host default.
- **Join capability:** supplied by the operator for this session; it is not
  saved by the app.

The view is the host's latest validated text projection. The native session
continues while the activity is open and stops when the user disconnects or
leaves the activity.
