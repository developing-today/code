{
  description = "atlas-search — Atlas corpus search CLI + service (developing-today/code#5350)";

  inputs = {
    # Track nixos-unstable, same as pkgs/id's sub-flake.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    systems = {
      url = "github:nix-systems/default";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.systems.follows = "systems";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        # The pinned toolchain (rust-toolchain.toml, channel
        # 1.99.0) — the same rustup toolchain the step 1-4 gates
        # ran under on the laptop, so the packaged build compiles
        # with the compiler the parity evidence belongs to.
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };
        atlas-search = pkgs.callPackage ./default.nix { inherit rustPlatform; };
      in
      {
        packages = {
          inherit atlas-search;
          default = atlas-search;
        };
        checks.default = atlas-search;
        devShells.default = pkgs.mkShell {
          packages = [ rustToolchain ];
        };
      }
    );
}
