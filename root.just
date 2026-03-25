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
alias fix := lockfiles

# =============================================================================
# Utilities
# =============================================================================

# Recursively chown all files (including hidden) to current user:group (requires sudo)
[group('util')]
chown:
    #!/usr/bin/env bash
    set -euxo pipefail
    id
    user="$(id -u)"
    group="$(id -g)"
    sudo chown -R "$user:$group" .
