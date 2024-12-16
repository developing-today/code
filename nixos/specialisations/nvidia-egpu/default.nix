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
  # specialisation = {
  #   nvidia-egpu.configuration = {
  system.nixos.tags = [
    # "nvidia"
    # "egpu"
    "nvidia-egpu"
  ];
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
        #           # ./lib/pci-to-int.sh
        amdgpuBusId = "PCI:193:0:0";
        nvidiaBusId = "PCI:100:0:0";
      };
    };
  };
  #   };
  # };
}
# {
#   pkgs,
#   inputs,
#   config,
#   ...
# }:
# {
#   specialisation = {
#     nvidia-egpu.configuration = {
#       # imports = [ inputs.nixos-hardware.nixosModules.common-gpu-nvidia ];
#       # boot.extraModulePackages = [ config.boot.kernelPackages.nvidia_x11 ];
#       # boot.blacklistedKernelModules = [ "nouveau" "amdgpu" ];
#       # boot.kernelPackages = pkgs.linuxPackages_6_1; # EOL 2026 | 2033 CIP # https://github.com/133760D/Nix-nvidia-configuration/blob/main/boot_kernel.nix
#       # boot.initrd.kernelModules = [
#       # "nvidia"
#       # "i915"
#       # "nvidia_modeset"
#       # "nvidia_uvm"
#       # "nvidia_dm"
#       # ];
#       # systemd.services.nvidia-persistenced = {
#       #   enable = true;
#       #   description = "NVIDIA Persistence Daemon";
#       #   after = [ "multi-user.target" ];
#       #   wantedBy = [ "multi-user.target" ];
#       # };
#       # environment.systemPackages = with pkgs; [
#       #     vulkan-tools
#       #     vulkan-loader
#       #     vulkan-validation-layers
#       #   ];
#       hardware.graphics = {
#         enable = true;
#         enable32Bit = true;
#         # extraPackages = with pkgs; [ vaapiVdpau ];
#       };
#       services.xserver.videoDrivers = [ "nvidia" ];
#       hardware.nvidia = {
#         modesetting.enable = true;
#         powerManagement.enable = false;
#         powerManagement.finegrained = false;
#         open = false; # true;
#         # # https://github.com/NixOS/nixpkgs/blob/nixos-unstable/pkgs/os-specific/linux/nvidia-x11/default.nix
#         package = config.boot.kernelPackages.nvidiaPackages.production;
#         # # nvidiaPersistenced = true;
#         prime = {
#           reverseSync.enable = true;
#           # ./lib/pci-to-int.sh
#           amdgpuBusId = "PCI:193:0:0";
#           nvidiaBusId = "PCI:100:0:0";
#         };
#       };
#     };
#   };
# }
