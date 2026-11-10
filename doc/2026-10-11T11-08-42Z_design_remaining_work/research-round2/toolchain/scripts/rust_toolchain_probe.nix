# Read-only eval: does a rust-overlay rev build a 1.99.0 toolchain with the repo's components, and what rustc does nixpkgs' rustPlatform use?
let
  ovRev = builtins.getEnv "OV_REV";
  npRev = builtins.getEnv "NP_REV";
  rustOverlay = builtins.getFlake "github:oxalica/rust-overlay/${ovRev}";
  nixpkgsFlake = builtins.getFlake "github:NixOS/nixpkgs/${npRev}";
  pkgs = import nixpkgsFlake.outPath { system = "x86_64-linux"; overlays = [ (import rustOverlay.outPath) ]; };
  tc = pkgs.rust-bin.fromRustupToolchain {
    channel = "1.99.0";
    profile = "default";
    components = [ "rustfmt" "clippy" "rust-analyzer" "llvm-tools" ];
  };
in
builtins.concatStringsSep "\n" [
  "rust-overlay=${ovRev} nixpkgs=${npRev}"
  "rust-bin 1.99.0 default(+repo components).version=${tc.version}"
  "nixpkgs.rustPlatform.rust.rustc.version=${pkgs.rustPlatform.rust.rustc.version}"
  "nixpkgs.rustPlatform.rust.cargo.version=${pkgs.rustPlatform.rust.cargo.version}"
  ""
]
