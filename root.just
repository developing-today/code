# Show available recipes
list:
  @just --list

# =============================================================================
# Flake Input Management
# =============================================================================

# Update a list of flake inputs by name (lock-only, no build)
[group('flake')]
update-input +inputs:
    #!/usr/bin/env bash
    set -euo pipefail
    for input in {{inputs}}; do
        echo "--- Updating $input ---"
        nix flake update "$input"
    done
alias update-inputs := update-input

# Update all flake inputs (lock-only, no build)
[group('flake')]
update-inputs-all:
    nix flake update
alias update-all-inputs := update-inputs-all

# =============================================================================
# NixOS/Nixpkgs Input Management
# =============================================================================

# Helpers to parse flake.lock via nix for NixOS/nixpkgs inputs
[private]
_nixpkgs-inputs ref="":
    #!/usr/bin/env bash
    if [ -z "{{ref}}" ]; then
        nix eval --raw --impure --expr 'import ./nix/nixpkgs-inputs.nix {}'
    else
        nix eval --raw --impure --expr "import ./nix/nixpkgs-inputs.nix { ref = \"{{ref}}\"; }"
    fi

# Update all direct NixOS/nixpkgs inputs (any branch)
[group('nixpkgs')]
update-nixpkgs-all:
    ./scripts/update-nixpkgs-inputs.sh

# Update NixOS/nixpkgs inputs on master (or no explicit branch)
[group('nixpkgs')]
update-nixpkgs-master:
    ./scripts/update-nixpkgs-inputs.sh master

# Update NixOS/nixpkgs inputs on nixos-unstable branch
[group('nixpkgs')]
update-nixpkgs-unstable:
    ./scripts/update-nixpkgs-inputs.sh nixos-unstable

# Update all NixOS/nixpkgs inputs by branch category (master then unstable)
[group('nixpkgs')]
update-nixpkgs:
    ./scripts/update-nixpkgs-inputs.sh master nixos-unstable

# Update all NixOS/nixpkgs inputs by name only (no URL discovery, just pass names through)
[group('nixpkgs')]
update-nixpkgs-all-only:
    #!/usr/bin/env bash
    set -euo pipefail
    inputs=$(just _nixpkgs-inputs)
    if [ -z "$inputs" ]; then
        echo "No NixOS/nixpkgs inputs found in flake.lock"
        exit 0
    fi
    just update-input $inputs

# Update NixOS/nixpkgs master inputs by name only
[group('nixpkgs')]
update-nixpkgs-master-only:
    #!/usr/bin/env bash
    set -euo pipefail
    inputs=$(just _nixpkgs-inputs master)
    if [ -z "$inputs" ]; then
        echo "No NixOS/nixpkgs master inputs found in flake.lock"
        exit 0
    fi
    just update-input $inputs

# Update NixOS/nixpkgs nixos-unstable inputs by name only
[group('nixpkgs')]
update-nixpkgs-unstable-only:
    #!/usr/bin/env bash
    set -euo pipefail
    inputs=$(just _nixpkgs-inputs nixos-unstable)
    if [ -z "$inputs" ]; then
        echo "No NixOS/nixpkgs nixos-unstable inputs found in flake.lock"
        exit 0
    fi
    just update-input $inputs

# =============================================================================
# Lockfiles
# =============================================================================

# Regenerate just-recipes.json from justfile for dynamic nix app generation
[group('deps')]
just-recipes:
    just --dump --dump-format json > just-recipes.json
    biome format --write --config-path . just-recipes.json

# Regenerate all lockfiles (just-recipes.json)
[group('deps')]
lockfiles: just-recipes

# Run treefmt (cached, finds nearest treefmt.toml from pwd)
[group('format')]
treefmt *args:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="$(pwd)"
    while [ "$dir" != "/" ] && [ ! -f "$dir/treefmt.toml" ]; do dir="$(dirname "$dir")"; done
    if [ ! -f "$dir/treefmt.toml" ]; then echo "error: no treefmt.toml found" >&2; exit 1; fi
    unset TREEFMT_TREE_ROOT TREEFMT_TREE_ROOT_CMD TREEFMT_TREE_ROOT_FILE
    exec treefmt --config-file "$dir/treefmt.toml" --tree-root "$dir" {{args}}

# Run treefmt without cache (finds nearest treefmt.toml from pwd)
[group('format')]
treefmt-no-cache *args:
    #!/usr/bin/env bash
    set -euo pipefail
    dir="$(pwd)"
    while [ "$dir" != "/" ] && [ ! -f "$dir/treefmt.toml" ]; do dir="$(dirname "$dir")"; done
    if [ ! -f "$dir/treefmt.toml" ]; then echo "error: no treefmt.toml found" >&2; exit 1; fi
    unset TREEFMT_TREE_ROOT TREEFMT_TREE_ROOT_CMD TREEFMT_TREE_ROOT_FILE
    exec treefmt --config-file "$dir/treefmt.toml" --tree-root "$dir" --no-cache {{args}}
alias treefmt-uncached := treefmt-no-cache

# Format all code (via treefmt)
[group('format')]
fmt *args: (treefmt args)

# Strip trailing whitespace from all tracked text files
[group('format')]
strip-whitespace:
    #!/usr/bin/env bash
    set -euo pipefail
    git grep -rl '[[:space:]]$' -- \
      ':!*.patch' ':!*.min.*' ':!*.lock' ':!*.map' ':!*.generated.*' ':!*.pb.*' ':!*.g.dart' \
      ':!*.exe' ':!*.dll' ':!*.so' ':!*.dylib' ':!*.a' ':!*.o' ':!*.obj' \
      ':!*.png' ':!*.jpg' ':!*.jpeg' ':!*.gif' ':!*.bmp' ':!*.ico' ':!*.webp' ':!*.svg' ':!*.tiff' ':!*.tif' ':!*.avif' \
      ':!*.mp4' ':!*.mkv' ':!*.avi' ':!*.mov' ':!*.wmv' ':!*.flv' ':!*.webm' \
      ':!*.mp3' ':!*.wav' ':!*.flac' ':!*.aac' ':!*.ogg' ':!*.wma' ':!*.m4a' ':!*.opus' \
      ':!*.zip' ':!*.tar' ':!*.gz' ':!*.bz2' ':!*.xz' ':!*.zst' ':!*.7z' ':!*.rar' \
      ':!*.pdf' ':!*.woff' ':!*.woff2' ':!*.ttf' ':!*.otf' ':!*.eot' \
      2>/dev/null | while IFS= read -r f; do
      case "$f" in
        */node_modules/*|*/dist/*|*/target/*|*/vendor/*|*/vendored/*|*/release/*|*/bin/*) continue ;;
        */build/*|*/out/*|*/output/*|*/_build/*|*/pkg/*|*/.next/*|*/.nuxt/*|*/.output/*) continue ;;
        */_site/*|*/.cache/*|*/__pycache__/*|*/.tox/*|*/_generated/*) continue ;;
        */third_party/*|*/third-party/*|*/external/*|*/deps/*|*/Pods/*|*/.bundle/*) continue ;;
        */bower_components/*|*/.pnp/*|*/.eggs/*|*.egg-info/*|*/site-packages/*) continue ;;
        */.gradle/*|*/.mvn/*|*/Packages/*) continue ;;
      esac
      case "$(file -b --mime-encoding "$f")" in binary) continue ;; esac
      sed -i 's/[[:space:]]*$//' "$f"
    done || true

# Run all fixers: strip whitespace, regenerate lockfiles, format
[group('format')]
fix *args: strip-whitespace lockfiles (fmt args)

# =============================================================================
# Utilities
# =============================================================================

# Reinstall/update lootbox binary from latest source
[group('util')]
update-lootbox:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Updating lootbox..."
    curl -fsSL https://raw.githubusercontent.com/jx-codes/lootbox/main/install.sh | bash
    echo "lootbox updated to $(lootbox --version)"

# Start lootbox server (if not already running)
[group('util')]
lootbox-server:
    #!/usr/bin/env bash
    set -euo pipefail
    if ss -tlnp 2>/dev/null | grep -q ':9420 .*lootbox'; then
        echo "Lootbox server already running on port 9420"
    else
        echo "Starting lootbox server..."
        nohup lootbox server --port 9420 &>/dev/null &
        disown
        sleep 1
        if ss -tlnp 2>/dev/null | grep -q ':9420 .*lootbox'; then
            echo "Lootbox server started on port 9420"
        else
            echo "Lootbox server may still be starting up..."
        fi
    fi

# Kill running lootbox server
[group('util')]
lootbox-kill:
    #!/usr/bin/env bash
    set -euo pipefail
    pid=$(ss -tlnp 2>/dev/null | grep ':9420 ' | grep -oP 'pid=\K[0-9]+' || true)
    if [ -n "$pid" ]; then
        kill "$pid"
        echo "Killed lootbox server (PID $pid)"
    else
        echo "No lootbox server running on port 9420"
    fi

# Recursively chown all files (including hidden) to current user:group (requires sudo)
[group('util')]
chown:
    #!/usr/bin/env bash
    set -euxo pipefail
    id
    user="$(id -u)"
    group="$(id -g)"
    sudo chown -R "$user:$group" .
