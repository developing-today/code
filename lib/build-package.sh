#!/usr/bin/env bash
nix-build -E "with import <nixpkgs> {}; callPackage ./package.nix {}"
