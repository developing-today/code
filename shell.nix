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
    (pkgs.python3.withPackages (python-pkgs: with python-pkgs; [
      pydbus
      dbus-python
      pygobject3
      gbulb
      dbus-python
      # python312Packages.pydbus
      # python312Packages.pygobject3
    ]))
  ] ++ [
    # dbus-python
    # pygobject3
    pkgs.gobject-introspection
    pkgs.glib
  ];
  shellHook = ''
    # Add any shell initialization commands here, for instance:
    echo "Welcome to the development shell!"
  '';
}
