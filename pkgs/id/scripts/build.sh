#!/usr/bin/env bash
# Conditional build script for id project
# Usage: build.sh [variant] [profile]
#   variant: lib | web (default: web)
#   profile: debug | release (default: debug)
#
# Tracks build variant in target/.build-variant[-release] to detect when
# a rebuild is needed due to variant change.

set -euo pipefail

VARIANT="${1:-web}"
PROFILE="${2:-debug}"

# Validate inputs
if [[ "$VARIANT" != "lib" && "$VARIANT" != "web" ]]; then
  echo "Error: variant must be 'lib' or 'web', got '$VARIANT'" >&2
  exit 1
fi

if [[ "$PROFILE" != "debug" && "$PROFILE" != "release" ]]; then
  echo "Error: profile must be 'debug' or 'release', got '$PROFILE'" >&2
  exit 1
fi

# Set paths based on profile
if [[ "$PROFILE" == "release" ]]; then
  BINARY="target/release/id"
  VARIANT_FILE="target/.build-variant-release"
  CARGO_FLAGS="--release"
else
  BINARY="target/debug/id"
  VARIANT_FILE="target/.build-variant"
  CARGO_FLAGS=""
fi

# ─────────────────────────────────────────────────────────────────────────────
# Step 1: Build web assets if needed (only for web variant)
# ─────────────────────────────────────────────────────────────────────────────
if [[ "$VARIANT" == "web" ]]; then
  needs_frontend=false

  if [[ ! -f web/dist/manifest.json ]]; then
    echo "[web] No manifest found, frontend build needed"
    needs_frontend=true
  else
    manifest_time=$(stat -c %Y web/dist/manifest.json 2>/dev/null || echo 0)

    # Use find for robust recursive file discovery
    # Check all .ts, .css, .json config files under web/ (excluding dist/ and node_modules/)
    # Also check bun.lock for dependency changes
    while IFS= read -r f; do
      if [[ -f "$f" ]]; then
        file_time=$(stat -c %Y "$f" 2>/dev/null || echo 0)
        if [[ "$file_time" -gt "$manifest_time" ]]; then
          echo "[web] $f is newer than manifest"
          needs_frontend=true
          break
        fi
      fi
    done < <(
      find web -type f \( -name '*.ts' -o -name '*.css' -o -name '*.json' -o -name 'bun.lock' \) \
        ! -path 'web/dist/*' ! -path 'web/node_modules/*' 2>/dev/null
    )
  fi

  if [[ "$needs_frontend" == "true" ]]; then
    echo "[web] Building frontend assets..."
    (cd web && bun install && bun run build)
  else
    echo "[web] Frontend assets up to date"
  fi
fi

# ─────────────────────────────────────────────────────────────────────────────
# Step 2: Build Rust binary if needed
# ─────────────────────────────────────────────────────────────────────────────
needs_backend=false
OTHER_VARIANT=$([[ "$VARIANT" == "web" ]] && echo "lib" || echo "web")

if [[ ! -f "$BINARY" ]]; then
  echo "[rust] No binary found, build needed"
  needs_backend=true
elif [[ -f "$VARIANT_FILE" ]] && [[ "$(cat "$VARIANT_FILE")" == "$OTHER_VARIANT" ]]; then
  echo "[rust] Last build was '$OTHER_VARIANT' variant, rebuilding as '$VARIANT'"
  needs_backend=true
else
  binary_time=$(stat -c %Y "$BINARY" 2>/dev/null || echo 0)

  # Use find for robust recursive file discovery
  while IFS= read -r f; do
    if [[ -f "$f" ]]; then
      file_time=$(stat -c %Y "$f" 2>/dev/null || echo 0)
      if [[ "$file_time" -gt "$binary_time" ]]; then
        echo "[rust] $f is newer than binary"
        needs_backend=true
        break
      fi
    fi
  done < <(
    find src -name '*.rs' -type f 2>/dev/null
    echo Cargo.toml
    echo Cargo.lock
    # For web variant, also check embedded assets
    if [[ "$VARIANT" == "web" ]]; then
      find web/dist -type f 2>/dev/null
    fi
  )
fi

if [[ "$needs_backend" == "true" ]]; then
  echo "[rust] Building $VARIANT $PROFILE variant..."

  if [[ "$VARIANT" == "web" ]]; then
    # Web is default, no extra flags needed
    cargo build $CARGO_FLAGS
  else
    # Lib variant disables default web feature
    cargo build $CARGO_FLAGS --no-default-features
  fi

  mkdir -p target
  echo "$VARIANT" >"$VARIANT_FILE"
else
  echo "[rust] $VARIANT $PROFILE variant up to date"
fi
