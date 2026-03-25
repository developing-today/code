# Show available recipes
default:
    @just --list

# Update a list of flake inputs by name (lock-only, no build)
update-input +inputs:
    #!/usr/bin/env bash
    set -euo pipefail
    for input in {{inputs}}; do
        echo "--- Updating $input ---"
        nix flake update "$input"
    done

# Alias for update-input
alias update-inputs := update-input

# Update all flake inputs (lock-only, no build)
update-inputs-all:
    nix flake update

# Alias for update-inputs-all
alias update-all-inputs := update-inputs-all

# Helpers to parse flake.lock via nix for NixOS/nixpkgs inputs
_nixpkgs-inputs ref="":
    #!/usr/bin/env bash
    if [ -z "{{ref}}" ]; then
        nix eval --raw --impure --expr 'import ./nix/nixpkgs-inputs.nix {}'
    else
        nix eval --raw --impure --expr "import ./nix/nixpkgs-inputs.nix { ref = \"{{ref}}\"; }"
    fi

# Update all direct NixOS/nixpkgs inputs (any branch)
update-nixpkgs-all:
    ./scripts/update-nixpkgs-inputs.sh

# Update NixOS/nixpkgs inputs on master (or no explicit branch)
update-nixpkgs-master:
    ./scripts/update-nixpkgs-inputs.sh master

# Update NixOS/nixpkgs inputs on nixos-unstable branch
update-nixpkgs-unstable:
    ./scripts/update-nixpkgs-inputs.sh nixos-unstable

# Update all NixOS/nixpkgs inputs by branch category (master then unstable)
update-nixpkgs: update-nixpkgs-master update-nixpkgs-unstable

# Update all NixOS/nixpkgs inputs by name only (no URL discovery, just pass names through)
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
update-nixpkgs-unstable-only:
    #!/usr/bin/env bash
    set -euo pipefail
    inputs=$(just _nixpkgs-inputs nixos-unstable)
    if [ -z "$inputs" ]; then
        echo "No NixOS/nixpkgs nixos-unstable inputs found in flake.lock"
        exit 0
    fi
    just update-input $inputs
