#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
ZIG="${ZIG:-zig}"
TARGET_DIR="targets/x64musl"
mkdir -p "$TARGET_DIR"

# Stages the static musl link pieces roc needs for x64musl targets, all
# produced by the local zig toolchain — nothing is downloaded. The pieces
# land in targets/x64musl/ (gitignored except libhost.a) and a fresh build
# of libhost.a follows, so `roc build --target=x64musl` works offline.
#
# With MUSL_PIECES=<dir> the four pieces are taken from there instead.
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
printf 'int main(){return 0;}\n' > "$scratch/tiny.c"
ZIG_GLOBAL_CACHE_DIR="$scratch/cache" "$ZIG" cc \
    -target x86_64-linux-musl -static "$scratch/tiny.c" -o "$scratch/tiny" >/dev/null
for piece in crt1.o libc.a libzigc.a libcompiler_rt.a; do
    if [ -n "${MUSL_PIECES:-}" ]; then
        cp "$MUSL_PIECES/$piece" "$TARGET_DIR/$piece"
    else
        found="$(find "$scratch/cache" -name "$piece" -type f | head -1)"
        if [ -z "$found" ]; then
            echo "could not stage $piece (set MUSL_PIECES to a directory with the four pieces)" >&2
            exit 1
        fi
        cp "$found" "$TARGET_DIR/$piece"
    fi
done

"$ZIG" build-lib -O ReleaseSmall -target x86_64-linux-musl -lc --name host -static \
    host/src/native_worker.zig -femit-bin="$TARGET_DIR/libhost.a"
echo "Built $TARGET_DIR/libhost.a (and the musl link pieces beside it)"
