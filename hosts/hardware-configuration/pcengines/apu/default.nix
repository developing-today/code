{
  lib,
  inputs,
  ...
}:
{
  imports = [
    (lib.from-root "hosts/hardware-configuration")
    inputs.nixos-hardware.nixosModules.pcengines-apu
  ];
}
