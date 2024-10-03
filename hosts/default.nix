inputs:
let
  lib = inputs.self.lib;
  host = lib.nixos-host-configuration;
in {
  nixos = host {
    profiles = "desktop";
    hardware = "framework/13-inch/12th-gen-intel";
  };
  amd = host {
    profiles = "desktop";
    hardware = "framework/13-inch/7040-amd";
  };
}
