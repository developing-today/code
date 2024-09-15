{
  config,
  lib,
  pkgs,
  modulesPath,
  ...
}:
{
  imports = [
    # NixOS/nixos-hardware
    (modulesPath + "/installer/scan/not-detected.nix")
    ../../../common
  ];
  boot.initrd.availableKernelModules = [
    "xhci_pci"
    "thunderbolt"
    "nvme"
    "usb_storage"
    "sd_mod"
  ];
  boot.kernelModules = [ "kvm-intel" ];
  powerManagement.cpuFreqGovernor = lib.mkDefault "performance";
  hardware.cpu.intel.updateMicrocode = lib.mkDefault config.hardware.enableRedistributableFirmware;
}
