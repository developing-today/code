{ lib }:
let
  nixos-host-configuration =
    options: name:
    lib.attrsets.recursiveUpdate rec {
      inherit name;
      type = name;
      system = "x86_64-linux";
      stateVersion = "23.11";
      group-key = lib.group-key name;
      email = "nixos-host-${name}@developing-today.com";
      sshKey = lib.host-key name;
      hardware = ./hosts/common/modules/hardware-configuration/common;
    } options;
in
{
  nixos = nixos-host-configuration {
    hardware = ./hosts/common/modules/hardware-configuration/framework/13-inch/12th-gen-intel;
  };
  amd = nixos-host-configuration {
    hardware = ./hosts/common/modules/hardware-configuration/framework/13-inch/7040-amd;
  };
}
