{
  config,
  lib,
  pkgs,
  modulesPath,
  ...
}:
{
  imports = [
    ../../../common
    # NixOS/nixos-hardware
  ];
  boot.initrd.availableKernelModules = [
    "xhci_pci"
    "nvme"
    "usb_storage"
    "sd_mod"
  ];
  # TODO
}
