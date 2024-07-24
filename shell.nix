{ pkgs ? import <nixpkgs> { } }:
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
  shellHook = ''
    # Add any shell initialization commands here, for instance:
    echo "Welcome to the development shell!"
  '';
}
