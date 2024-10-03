{ lib }:
rec {
  nixos = lib.nixos-host-configuration {
    hardware = "framework/13-inch/12th-gen-intel";
    # hardware = ./hosts/common/modules/hardware-configuration/framework/13-inch/12th-gen-intel;
  };
  amd = lib.nixos-host-configuration {
    hardware = "framework/13-inch/7040-amd";
    # hardware = ./hosts/common/modules/hardware-configuration/framework/13-inch/7040-amd;
  };
}
