{ inputs, lib, ... }:
{
  nix = {
    daemonCPUSchedPolicy = "idle";
    settings = {
      trusted-users = [
        "root"
        "@wheel"
      ];
      # auto-optimise-store = lib.mkDefault true;
      experimental-features = [
        "nix-command"
        "flakes"
        "repl-flake"
        "ca-derivations"
      ];
      warn-dirty = false;
      builders-use-substitutes = true;

      substituters = [
        "https://nix-community.cachix.org"
        "https://cuda-maintainers.cachix.org"
        "https://anyrun.cachix.org"
      ];
      trusted-public-keys = [
        "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
        "cuda-maintainers.cachix.org-1:0dq3bujKpuEPMCX6U4WylrUDZ9JyUG0VpVZa7CNfq5E="
        "anyrun.cachix.org-1:pqBobmOjI7nKlsUMV25u9QHa9btJK65/C8vnO3p346s="
      ];
      flake-registry = ""; # Disable global flake registry

      use-xdg-base-directories = true;
    };

    # Add each flake input as a registry
    # To make nix3 commands consistent with the flake
    registry = lib.mapAttrs (_name: v: { flake = v; }) inputs;

    # Add nixpkgs input to NIX_PATH
    # This lets nix2 commands still use <nixpkgs>
    nixPath = [ "nixpkgs=${inputs.nixpkgs.outPath}" ];
  };
}
# {
#   inputs,
#   lib,
#   pkgs,
#   ...
# }:
# let
#   flakeInputs = lib.filterAttrs (_: lib.isType "flake") inputs;
# in
# {
#   nix = {
#     package = pkgs.nixVersions.nix_2_22;

#     settings = {
#       extra-substituters = lib.mkAfter [ "https://cache.m7.rs" ];
#       extra-trusted-public-keys = [ "cache.m7.rs:kszZ/NSwE/TjhOcPPQ16IuUiuRSisdiIwhKZCxguaWg=" ];
#       trusted-users = [
#         "root"
#         "@wheel"
#       ];
#       auto-optimise-store = lib.mkDefault true;
#       experimental-features = [
#         "nix-command"
#         "flakes"
#         "ca-derivations"
#       ];
#       warn-dirty = false;
#       system-features = [
#         "kvm"
#         "big-parallel"
#         "nixos-test"
#       ];
#       flake-registry = ""; # Disable global flake registry
#     };
#     gc = {
#       automatic = true;
#       dates = "weekly";
#       # Keep the last 3 generations
#       options = "--delete-older-than +3";
#     };

#     # Add each flake input as a registry and nix_path
#     registry = lib.mapAttrs (_: flake: { inherit flake; }) flakeInputs;
#     nixPath = lib.mapAttrsToList (n: _: "${n}=flake:${n}") flakeInputs;
#   };
# }
