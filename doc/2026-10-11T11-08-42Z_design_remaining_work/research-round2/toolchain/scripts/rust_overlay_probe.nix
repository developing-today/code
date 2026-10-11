# Read-only probe: which Rust stable does a given rust-overlay rev provide, and what rustc does a given nixpkgs rev provide.
# Revisions come from the environment (OV_REV, NP_REV) so no repo file is touched.
let
  ovRev = builtins.getEnv "OV_REV";
  npRev = builtins.getEnv "NP_REV";
  rustOverlay = builtins.getFlake "github:oxalica/rust-overlay/${ovRev}";
  nixpkgsFlake = builtins.getFlake "github:NixOS/nixpkgs/${npRev}";
  pkgs = import nixpkgsFlake.outPath {
    system = "x86_64-linux";
    overlays = [ (import rustOverlay.outPath) ];
  };
  has = v: if pkgs.rust-bin.stable ? ${v} then "present" else "absent";
in
builtins.concatStringsSep "\n" [
  "rust-overlay=${ovRev}"
  "nixpkgs=${npRev}"
  "rust-bin.stable.latest=${pkgs.rust-bin.stable.latest.rustc.version}"
  "rust-bin.stable.1.99.0=${has "1.99.0"}"
  "rust-bin.stable.1.97.0=${has "1.97.0"}"
  "nixpkgs.rustc=${pkgs.rustc.version}"
  "nixpkgs.cargo=${pkgs.cargo.version}"
  ""
]
