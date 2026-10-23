{ lib, inputs, ... }:
{
  imports = [
    (lib.from-root "nixos/hardware-configuration")
    inputs.nixos-hardware.nixosModules.pcengines-apu
  ];
}
