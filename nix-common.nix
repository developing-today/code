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
    sops
    ssh-to-age
    gnupg
    age
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
    # Save devshell PATH, then source HM session vars (EDITOR, etc.)
    _devshell_path="$PATH"
    unset __HM_SESS_VARS_SOURCED
    . "$HOME/.profile" 2>/dev/null || true
    # .profile prepends HM sessionPath entries to PATH — move devshell paths back to front
    # so devshell takes priority, with HM paths (e.g. ~/.local/bin) as fallback
    _hm_prefix="''${PATH%:$_devshell_path}"
    if [ "$_hm_prefix" != "$PATH" ]; then
      export PATH="$_devshell_path:$_hm_prefix"
    fi
    unset _devshell_path _hm_prefix

    echo "Welcome to the development shell!"
  '';
}
