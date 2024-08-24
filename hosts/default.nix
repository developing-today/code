{ inputs, outputs, lib, pkgs, ... }: { } // (import ./nixos) { inherit inputs outputs lib pkgs; }
