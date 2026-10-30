#!/usr/bin/env bash
# Provision @libsql/client locally without network access.
#
# `bun install` needs the npm registry, which this laptop can't reach reliably
# (it hangs on "Resolving dependencies"). A complete @libsql/client@0.18.0 tree
# already exists in the opencode-mem package cache, so this script copies the
# needed packages from there into ./node_modules. When network is available,
# `bun install` in this directory is the normal path instead.
set -euo pipefail
cd "$(dirname "$0")/.."

SRC="${LIBXML_SRC:-$HOME/.cache/opencode/npm/opencode-mem@latest/1792868233137/node_modules}"

if [ -d "$SRC/@libsql/client" ]; then
  echo "[setup] copying @libsql/client deps from opencode-mem cache"
  rm -rf node_modules/@libsql node_modules/@neon-rs node_modules/libsql node_modules/js-base64 node_modules/promise-limit node_modules/ws node_modules/detect-libc
  mkdir -p node_modules/@libsql
  cp -r "$SRC/@libsql/client" node_modules/@libsql/
  cp -r "$SRC/@libsql/core" node_modules/@libsql/
  cp -r "$SRC/@libsql/hrana-client" node_modules/@libsql/
  cp -r "$SRC/@libsql/isomorphic-ws" node_modules/@libsql/
  cp -r "$SRC/@libsql/linux-x64-gnu" node_modules/@libsql/
  cp -r "$SRC/@libsql/linux-x64-musl" node_modules/@libsql/
  cp -r "$SRC/libsql" node_modules/
  cp -r "$SRC/js-base64" node_modules/
  cp -r "$SRC/promise-limit" node_modules/
  cp -r "$SRC/ws" node_modules/
  cp -r "$SRC/detect-libc" node_modules/
  mkdir -p node_modules/@neon-rs
  cp -r "$SRC/@neon-rs/load" node_modules/@neon-rs/
  mkdir -p node_modules/@types
  cp -r "$SRC/@types/node" node_modules/@types/
  echo "[setup] done"
else
  echo "[setup] opencode-mem cache not found at $SRC — falling back to bun install"
  bun install
fi
