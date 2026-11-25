#!/usr/bin/env bash
# Slice-1 Android gate for the id-client facade (#7938 child #13403).
# Pass/fail: host tests (unit + dependency boundary + authenticated
# session), then the Android arm64-v8a target resolve.
#
# Toolchain notes (verified 2026-11-23 on this host):
# - Host checks use the ambient `cargo` (nix 1.99.0 here) with NO `RUSTC`
#   override: exporting a mismatched `RUSTC` poisons proc-macro reuse
#   (E0514) by mixing the ambient cargo with another toolchain's rustc.
# - The Android check uses `rustup run <toolchain> cargo` with an
#   explicit RUSTC scoped to that invocation only: the bare `rustc` on
#   PATH belongs to a different sysroot and reports the android target
#   as missing (E0463). The user-wide sccache wrapper
#   (~/.cargo/config.toml `build.rustc-wrapper`) is bypassed there via
#   a passthrough wrapper so the 1.99.1 target build never shares cache
#   entries with the ambient 1.99.0 host build.
# - The `ring`/`aws-lc-sys` build scripts need the NDK clang:
#   ANDROID_NDK_HOME must point at an installed NDK (e.g.
#   ~/.local/share/android-sdk/ndk/27.0.12077973); cc-rs derives
#   `aarch64-linux-android<api>-clang` from it. Without it the target
#   check fails at C compilation, not at Rust resolution.
# Argument contract (#3352): this gate takes no arguments. Any
# argument is a usage error naming the offender (exit 2), checked
# before any subprocess, network, or file write.
set -euo pipefail

if [ "$#" -gt 0 ]; then
  echo "usage: check-android.sh (takes no arguments)" >&2
  echo "check-android.sh: usage error: unexpected argument: $1" >&2
  exit 2
fi

cd "$(dirname "$0")/.."

TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.99.1}"

echo "== host: id-client tests (boundary + session) =="
env -u RUSTC cargo test -p id-client

echo "== android target resolve: aarch64-linux-android =="
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  for candidate in "$HOME/.local/share/android-sdk/ndk/"*/; do
    if [ -x "${candidate}toolchains/llvm/prebuilt/linux-x86_64/bin/clang" ]; then
      export ANDROID_NDK_HOME="$candidate"
      break
    fi
  done
fi
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  echo "FAIL: ANDROID_NDK_HOME is unset and no NDK found under ~/.local/share/android-sdk/ndk/"
  echo 'install one with: sdkmanager --install "ndk;27.0.12077973"'
  exit 1
fi
echo "NDK: $ANDROID_NDK_HOME"
# cc-rs (used by ring/aws-lc-sys build scripts) does not derive the NDK
# clang from ANDROID_NDK_HOME; it needs CC/CXX/AR for the target triple.
# API 21 is the arm64-v8a minimum and keeps slice 1 compatible with the
# widest device range.
NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin"
export TARGET_CC="$NDK_BIN/aarch64-linux-android21-clang"
export TARGET_CXX="$NDK_BIN/aarch64-linux-android21-clang++"
export TARGET_AR="$NDK_BIN/llvm-ar"
# Passthrough wrapper: `exec`s the real compiler with no caching, so the
# Android-target build under toolchain $TOOLCHAIN cannot read or write
# sccache entries owned by the ambient host toolchain.
ID_CLIENT_PASSTHROUGH="$(mktemp)"
printf '#!/usr/bin/env bash\nexec "$@"\n' >"$ID_CLIENT_PASSTHROUGH"
chmod +x "$ID_CLIENT_PASSTHROUGH"
# Isolated target dir: the 1.99.1 android-resolve build (including its
# host proc-macros) must not share target/ with the ambient 1.99.0 host
# build — sharing produces E0514 cross-version reuse errors in both
# directions and confuses sccache. Verified 2026-11-23.
export CARGO_TARGET_DIR="$PWD/target/android-check"
export RUSTC="${RUSTC:-$(rustup which --toolchain "$TOOLCHAIN" rustc)}"
CARGO_BUILD_RUSTC_WRAPPER="$ID_CLIENT_PASSTHROUGH" \
  rustup run "$TOOLCHAIN" cargo check -p id-client --target aarch64-linux-android
rm -f "$ID_CLIENT_PASSTHROUGH"
