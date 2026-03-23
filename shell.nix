# Nix shell environment for the project.
#
# This shell.nix uses the same shared configuration as flake.nix
# via nix-common.nix for consistent environments.
#
# Usage:
#   nix-shell                      # Enter development environment
#   nix-shell --pure               # Enter isolated environment
#
# For flake users: `nix develop` provides an equivalent environment.

{
  pkgs ? import <nixpkgs> { },
}:
let
  # Import shared configuration
  nixCommon = import ./nix-common.nix { inherit pkgs; };
in
pkgs.mkShell {
  inherit (nixCommon)
    NIX_CONFIG
    nativeBuildInputs
    packages
    shellHook
    ;
}
