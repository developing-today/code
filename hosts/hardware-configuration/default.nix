{ lib, modulesPath, ... }:
{
  imports = [ (modulesPath + "/installer/scan/not-detected.nix") ];
  swapDevices = [
    {
      device = "/swapfile";
      size = 1024 * 192;
    }
    # "/dev/disk/by-label/NIXSWAP" # TODO: Add swap partition
    # zram? zswap?
  ];
  networking.useDHCP = lib.mkDefault true;
  powerManagement.cpuFreqGovernor = lib.mkDefault "performance";
  nixpkgs.hostPlatform = lib.mkDefault "x86_64-linux";
  boot.initrd.kernelModules = [ ];
  boot.extraModulePackages = [ ];
  # security.pam.loginLimits = [
  #   {
  #     domain = "@wheel";
  #     item = "nofile";
  #     type = "soft";
  #     value = "524288";
  #   }
  #   {
  #     domain = "@wheel";
  #     item = "nofile";
  #     type = "hard";
  #     value = "1048576";
  #   }
  # ];
  # devmon.enable = true;
  # udisks2.enable = true;
  # gvfs.enable = true;
  # pkgs linux kernel
}
