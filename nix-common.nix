# Shared Nix configuration for shell.nix and flake.nix
#
# This file ensures both environments have identical packages, environment
# variables, and shell hooks. The flake.lock provides exact version pinning.
#
# Usage in flake.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; };
#
# Usage in shell.nix:
#   nixCommon = import ./nix-common.nix { inherit pkgs; };

{ pkgs }:

let
  # Rust toolchain from rust-toolchain.toml (includes rustc, cargo, rustfmt, clippy)
  # Works because both shell.nix and flake.nix apply rust-overlay before importing
  # Uses pkgs/id path directly (root symlink not tracked by git, won't be in Nix store)
  rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./pkgs/id/rust-toolchain.toml;

  # Formatter binaries (keep in sync with treefmt.toml)
  # Used by formatter wrapper (nix fmt), nix flake checks, and devShell PATH
  fmtBins = [
    # Rust toolchain (includes rustfmt)
    rustToolchain
  ]
  ++ (with pkgs; [
    # Formatter orchestrator
    treefmt
    # Formatters and linters (keep in sync with treefmt.toml)
    nixfmt
    statix
    biome
    nodePackages.prettier
    shfmt
    shellcheck
    ruff
    rufo
    elmPackages.elm-format
    go
    haskellPackages.ormolu
    taplo
    # Utilities needed by formatter wrapper
    file
    just
    gnused
    findutils
    bash
  ]);

  # Native build inputs (tools) — used by nativeBuildInputs and packages
  nativeBuildInputs =
    fmtBins
    ++ (with pkgs; [
      # Build dependencies (Rust compilation)
      pkg-config
      openssl

      nix
      home-manager
      git
      sops
      ssh-to-age
      gnupg
      age

      # Python/uv (used by scripts)
      uv

      # Manual linters (not in treefmt, run manually)
      deadnix
    ]);
in
{
  inherit rustToolchain fmtBins nativeBuildInputs;

  NIX_CONFIG = "extra-experimental-features = nix-command flakes ca-derivations";
  # Anchor bare treefmt to current directory (for ad-hoc devshell use)
  # (just recipes and nix fmt use explicit --config-file/--tree-root instead)
  TREEFMT_TREE_ROOT_CMD = "pwd";

  # Build inputs (libraries for Rust compilation)
  buildInputs = with pkgs; [ openssl ];

  # OpenSSL environment variables
  opensslEnv = {
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
    OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";
    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
  };

  # Packages for shell.nix / nix develop (nativeBuildInputs + extra runtime deps)
  packages = nativeBuildInputs ++ [
    (pkgs.python3.withPackages (
      python-pkgs: with python-pkgs; [
        pydbus
        dbus-python
        pygobject3
        dbus-python
      ]
    ))
    pkgs.gobject-introspection
    pkgs.glib
  ];

  # Shared shell hook for both nix-shell and nix develop
  shellHook = ''
    # nix develop reconstructs PATH, losing HM session paths.
    # Restore them: devshell → HM → system
    # Guard: only do PATH reordering once (multiple use flake in .envrc)
    if [ -z "$__DEVSHELL_HM_PATH_DONE" ]; then
      export __DEVSHELL_HM_PATH_DONE=1

      # Source HM session vars and extract any PATH additions
      _pre_profile_path="$PATH"
      unset __HM_SESS_VARS_SOURCED
      . "$HOME/.profile" 2>/dev/null || true
      _hm_paths="''${PATH%:$_pre_profile_path}"
      [ "$_hm_paths" = "$PATH" ] && _hm_paths=""

      # Separate devshell from system using NIX_PROFILES (NixOS env var)
      # Non-NixOS: NIX_PROFILES is unset, everything stays in devshell, PATH unchanged
      _devshell_only=""
      _system_only=""
      _remaining="$_pre_profile_path"
      while [ -n "$_remaining" ]; do
        _e="''${_remaining%%:*}"
        [ "$_e" = "$_remaining" ] && _remaining="" || _remaining="''${_remaining#*:}"
        _is_sys=0
        for _p in $NIX_PROFILES; do
          case "$_e" in "$_p"/*) _is_sys=1; break ;; esac
        done
        if [ "$_is_sys" = 1 ]; then
          _system_only="$_system_only''${_system_only:+:}$_e"
        else
          _devshell_only="$_devshell_only''${_devshell_only:+:}$_e"
        fi
      done

      # Reconstruct: devshell → HM → system
      export PATH="$_devshell_only''${_hm_paths:+:$_hm_paths}''${_system_only:+:$_system_only}"
      unset _pre_profile_path _hm_paths _devshell_only _system_only _remaining _e _is_sys _p
    fi
    echo ""
    just --list
    echo ""
    echo "Welcome to the development shell!"
    echo ""
  '';
}
