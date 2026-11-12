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
    # Crane: splits dependency compilation out of the package build
    # so workspace-source changes reuse the dependency layer
    # (code#8392). Pinned to the rev the code repo's root lock
    # already carries.
    crane = {
      url = "github:ipetkov/crane/01bc1d404a51a0a07e9d8759cd50a7903e218c82";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
      crane,
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
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;
        atlas-search = pkgs.callPackage ./default.nix { inherit rustPlatform craneLib; };
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
