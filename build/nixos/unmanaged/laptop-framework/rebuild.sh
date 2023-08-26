cd flakes/home || exit 1
nix flake update
cd ../.. || exit 1
nix flake update
git add .
sudo nixos-rebuild switch
