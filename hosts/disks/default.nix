{
  inputs,
  host,
  lib,
  ...
}:
{
  imports = [
    inputs.disko.nixosModules.disko
  ] ++ lib.make-disks host.disks;
}
