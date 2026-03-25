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

{
  NIX_CONFIG = "extra-experimental-features = nix-command flakes ca-derivations";

  # Native build inputs (tools)
  nativeBuildInputs = with pkgs; [
    nix
    home-manager
    git
    just
    sops
    ssh-to-age
    gnupg
    age

    # Python/uv (used by scripts)
    uv

    # Formatters and linters (keep in sync with treefmt.toml)
    treefmt
    nixfmt
    statix
    deadnix
    nodePackages.prettier
    shfmt
    rustfmt
    shellcheck
    ruff
    biome
    rufo
    elmPackages.elm-format
    go
    haskellPackages.ormolu
  ];

  # Additional packages
  packages = [
    (pkgs.python3.withPackages (
      python-pkgs: with python-pkgs; [
        pydbus
        dbus-python
        pygobject3
        dbus-python
      ]
    ))
  ]
  ++ [
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
