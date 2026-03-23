{
  pkgs ? import <nixpkgs> { },
}:
pkgs.mkShell {
  NIX_CONFIG = "extra-experimental-features = nix-command flakes ca-derivations";
  nativeBuildInputs = with pkgs; [
    nix
    home-manager
    git
    sops
    ssh-to-age
    gnupg
    age
  ];
  packages = [
    (pkgs.python3.withPackages (
      python-pkgs: with python-pkgs; [
        pydbus
        dbus-python
        pygobject3
        # gbulb
        dbus-python
        # python312Packages.pydbus
        # python312Packages.pygobject3
      ]
    ))
  ]
  ++ [
    # dbus-python
    # pygobject3
    pkgs.gobject-introspection
    pkgs.glib
  ];
  shellHook = ''
    # Save devshell PATH, then source HM session vars (EDITOR, etc.)
    _devshell_path="$PATH"
    unset __HM_SESS_VARS_SOURCED
    . "$HOME/.profile"
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
