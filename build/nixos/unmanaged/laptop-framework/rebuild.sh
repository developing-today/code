#!/usr/bin/env bash

git add .
cd flakes/home || exit 1
nix flake update
cd ../.. || exit 1
git add .
nix flake update
git add .
sudo nixos-rebuild switch
