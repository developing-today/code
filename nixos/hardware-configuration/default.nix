{ lib, modulesPath, ... }:
{
  imports = [ (modulesPath + "/installer/scan/not-detected.nix") ];
  # swapDevices = [ # TODO
  #   {
  #     device = "/swapfile";
  #     size = 1024 * 192;
  #   }
  #   # "/dev/disk/by-label/NIXSWAP" # TODO: Add swap partition
  #   # zram? zswap?
  # ];
  networking.useDHCP = lib.mkDefault true;
  nixpkgs.hostPlatform = lib.mkDefault "x86_64-linux";
  boot.initrd.kernelModules = [ ];
  boot.kernelModules = [
    "kvm-amd"
    "kvm-intel"
  ];
  boot.extraModulePackages = [ ];
  powerManagement.enable = true;
  powerManagement.powertop.enable = true;
  powerManagement.cpuFreqGovernor = lib.mkDefault "performance";
  programs.gamemode.enable = true;
  services.power-profiles-daemon.enable = true;
  services.thermald.enable = true;
  # services.power-profiles-daemon.enable = false;
  # services.auto-cpufreq.enable = true;
  # services.auto-cpufreq.settings = {
  #   battery = {
  #     governor = "powersave";
  #     turbo = "never";
  #   };
  #   charger = {
  #     governor = "performance";
  #     turbo = "auto";
  #   };
  # };
  # services.tlp = {
  #   enable = true;
  #   settings = {
  #     CPU_BOOST_ON_AC = 1;
  #     CPU_BOOST_ON_BAT = 0;
  #     CPU_SCALING_GOVERNOR_ON_AC = "performance";
  #     # CPU_SCALING_GOVERNOR_ON_BAT = "performance"; # Use "powersave" for better battery
  #     CPU_SCALING_GOVERNOR_ON_BAT = "powersave";
  #     STOP_CHARGE_THRESH_BAT0 = 95;
  #     CPU_ENERGY_PERF_POLICY_ON_AC = "performance";
  #     CPU_ENERGY_PERF_POLICY_ON_BAT = "power";
  #   };
  # };
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
