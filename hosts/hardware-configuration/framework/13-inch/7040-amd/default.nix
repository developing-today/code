{
  config,
  lib,
  pkgs,
  modulesPath,
  ...
}:
{
  imports = [
    (lib.from-root "hosts/hardware-configuration")
    # NixOS/nixos-hardware
  ];
  boot.initrd.availableKernelModules = [
    "xhci_pci"
    "nvme"
    "usb_storage"
    "sd_mod"
  ];
  # TODO
  # services.videoDrivers = [ "nvidia" ]; # If you are using a hybrid laptop add its iGPU manufacturer nvidia amd intel
}
