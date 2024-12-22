{ config, pkgs, ... }:
let
  nvidia-offload = pkgs.writeShellScriptBin "nvidia-offload" ''
    export __NV_PRIME_RENDER_OFFLOAD=1
    export __NV_PRIME_RENDER_OFFLOAD_PROVIDER=NVIDIA-GO
    export __GLX_VENDOR_LIBRARY_NAME=nvidia
    export __VK_LAYER_NV_optimus=NVIDIA_only
    exec "$@"
  '';
in
{
  # specialisation = { nvidia-egpu.configuration = {
  # system.nixos.tags = [
  # "nvidia"
  # "egpu"
  # "nvidia-egpu"
  # ];
  environment.systemPackages = [
    nvidia-offload
    pkgs.glxinfo
  ];
  services.xserver.videoDrivers = [ "nvidia" ];
  boot = {
    extraModprobeConfig = "options nvidia-drm modeset=1";
    initrd.kernelModules = [ "nvidia_modeset" ];
    #blacklistedKernelModules = [ "nouveau" ];
  };
  systemd.services.systemd-udev-trigger.restartIfChanged = false;
  hardware = {
    graphics = {
      enable = true;
      enable32Bit = true;
    };
    nvidiaOptimus.disable = false;
    nvidia = {
      package = config.boot.kernelPackages.nvidiaPackages.stable;
      modesetting.enable = true;
      powerManagement = {
        enable = false;
        finegrained = false;
      };
      open = true;
      nvidiaSettings = true;
      prime = {
        offload = {
          enable = true;
          enableOffloadCmd = true;
        };
        sync.enable = false;
        # ./lib/pci-to-int.sh
        amdgpuBusId = "PCI:193:0:0";
        nvidiaBusId = "PCI:100:0:0";
      };
    };
  };
  # };};
}
