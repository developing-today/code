# Nix shell environment for the id project.
#
# This shell.nix uses the exact same versions as flake.nix by reading
# the flake.lock file for reproducible builds without requiring flakes.
#
# Usage:
#   nix-shell                      # Enter development environment
#   nix-shell --pure               # Enter isolated environment
#   nix-shell --run "just test"    # Run tests
#   nix-shell --run "just check-all" # Run all checks
#
# For flake users: `nix develop` provides an equivalent environment.

let
  # Read flake.lock to get exact versions
  flakeLock = builtins.fromJSON (builtins.readFile ./flake.lock);

  # Extract locked versions from flake.lock
  nixpkgsLock = flakeLock.nodes.nixpkgs.locked;
  rustOverlayLock = flakeLock.nodes.rust-overlay.locked;

  # Fetch nixpkgs with exact hash from flake.lock
  nixpkgs = fetchTarball {
    url = "https://github.com/${nixpkgsLock.owner}/${nixpkgsLock.repo}/archive/${nixpkgsLock.rev}.tar.gz";
    sha256 = nixpkgsLock.narHash;
  };

  # Fetch rust-overlay with exact hash from flake.lock
  rustOverlay = fetchTarball {
    url = "https://github.com/${rustOverlayLock.owner}/${rustOverlayLock.repo}/archive/${rustOverlayLock.rev}.tar.gz";
    sha256 = rustOverlayLock.narHash;
  };

  pkgs = import nixpkgs {
    overlays = [ (import rustOverlay) ];
  };

  # Rust toolchain from rust-toolchain.toml (same as flake)
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

  # Import shared configuration
  nixCommon = import ./nix-common.nix { inherit pkgs; };

in
pkgs.mkShell {
  name = "id-dev";

  inherit (nixCommon) buildInputs shellHook;

  nativeBuildInputs = [ rustToolchain ] ++ nixCommon.nativeBuildInputs;

  # OpenSSL configuration for native builds
  inherit (nixCommon.opensslEnv)
    OPENSSL_DIR
    OPENSSL_LIB_DIR
    OPENSSL_INCLUDE_DIR
    PKG_CONFIG_PATH
    ;
}
