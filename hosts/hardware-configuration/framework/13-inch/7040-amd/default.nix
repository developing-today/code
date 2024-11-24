{ lib, inputs, ... }:
{
  imports = [
    (lib.from-root "hosts/hardware-configuration")
    inputs.nixos-hardware.nixosModules.framework-13-7040-amd
  ];
  boot.initrd.availableKernelModules = [
    "xhci_pci"
    "nvme"
    "usb_storage"
    "sd_mod"
     "r8152" "usbnet"
  ];
}
#     nvidia = {
#       # Enable modesetting for Wayland compositors (hyprland)
#       modesetting.enable = true;
#       # Use the open source version of the kernel module (for driver 515.43.04+)
#       open = true;
#       # Enable the Nvidia settings menu
#       nvidiaSettings = true;
#       # Select the appropriate driver version for your specific GPU
#       package = config.boot.kernelPackages.nvidiaPackages.stable;
#     };
#     opengl = { # for nvidia
#       enable = true;
#       driSupport = true;
#       driSupport32Bit = true;
#     };
# services.videoDrivers = [ "nvidia" ]; # If you are using a hybrid laptop add its iGPU manufacturer nvidia amd intel
#fwupd.enable = true; # laptop-framework # don't follow this guide you have a framework 12 intel # https://github.com/NixOS/nixos-hardware/tree/master/framework/13-inch/13th-gen-intel#getting-the-fingerprint-sensor-to-work
# https://knowledgebase.frame.work/ubuntu-fingerprint-troubleshooting-r1_DA0TMn
# TODO: pull the hardware flake for 12th gen intel
# nixos-hardware.nixosModules.framework-12th-gen-intel
