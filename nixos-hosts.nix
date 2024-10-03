inputs:
let
  lib = inputs.self.lib;
in {
  nixos = lib.nixos-host-configuration {
    profiles = "desktop";
    hardware = "framework/13-inch/12th-gen-intel";
  };
  amd = lib.nixos-host-configuration {
    profiles = "desktop";
    hardware = "framework/13-inch/7040-amd";
  };
}
