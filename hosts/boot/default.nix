{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  ...
}:
{
  boot = {
    # kernelPackages = pkgs.linuxKernel.packages.linux_
    tmp = {
      cleanOnBoot = true;
    };
    loader = {
      # grub = {
      #   enable = true;
      #   efiSupport = true;
      #   device = "nodev";
      #  # For installing with GRUB, mount your ESP to /boot/efi rather than /boot
      # };
      systemd-boot = {
        enable = true;
        configurationLimit = 2048;
      };
      efi = {
        canTouchEfiVariables = true;
        # efiSysMountPoint = "/boot/efi";
      };
    };
  };
}
