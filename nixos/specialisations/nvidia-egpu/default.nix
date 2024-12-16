{
  pkgs,
  inputs,
  config,
  ...
}:
{
  specialisation = {
    nvidia-egpu.configuration = {

      hardware.graphics.enable = true;

      services.xserver.videoDrivers = [ "nvidia" ];
      hardware.nvidia.open = true; # Set to false for proprietary drivers
      # imports = [ inputs.nixos-hardware.nixosModules.common-gpu-nvidia ];
      # # boot.kernelPackages = pkgs.linuxPackages_6_1; # EOL 2026 | 2033 CIP # https://github.com/133760D/Nix-nvidia-configuration/blob/main/boot_kernel.nix
      # # boot.initrd.kernelModules = [
      # # "nvidia"
      # # "i915"
      # # "nvidia_modeset"
      # # "nvidia_uvm"
      # # "nvidia_drm"
      # # ];

      # # boot.blacklistedKernelModules = [ "nouveau" ];
      # # boot.kernelParams = [ "module_blacklist=amdgpu" ];
      # # boot.extraModulePackages = [ config.boot.kernelPackages.nvidia_x11 ];
      # system.nixos.tags = [
      #   "nvidia"
      #   "egpu"
      # ];
      # hardware.graphics = {
      #   enable = true;
      #   enable32Bit = true;
      #   # extraPackages = with pkgs; [ vaapiVdpau ];

      #   # opengl.extraPackages32 = with pkgs.pkgsi686Linux; [ libva ];
      #   # opengl.setLdLibraryPath = true;
      # };
      # # environment.systemPackages = with pkgs; [
      # #     vulkan-tools
      # #     vulkan-loader
      # #     vulkan-validation-layers
      # #   ];
      # # hardware.opengl = {
      # #   enable = true;
      # #   driSupport = true;
      # #   driSupport32Bit = true;
      # # };
      # # nvidia-x11, nvidia-settings, and nvidia-persistenced.
      # services.xserver.videoDrivers = [ "nvidia" ];
      # # systemd.services.nvidia-persistenced = {
      # #   enable = true;
      # #   description = "NVIDIA Persistence Daemon";
      # #   after = [ "multi-user.target" ];
      # #   wantedBy = [ "multi-user.target" ];
      # # };
      # # environment.systemPackages = with pkgs; [
      # #   # nvidia-settings
      # #   nvidia-x11
      # # ];
      # hardware.nvidia = {
      #   #
      #   # export __NV_PRIME_RENDER_OFFLOAD=1
      #   # export __NV_PRIME_RENDER_OFFLOAD_PROVIDER=NVIDIA-G0
      #   # export __GLX_VENDOR_LIBRARY_NAME=nvidia
      #   # export __VK_LAYER_NV_optimus=NVIDIA_only
      #   # exec "$@"
      #   #               offload = {
      #   # 	enable = true;
      #   # 	enableOffloadCmd = true;
      #   # };
      #   # forceFullCompositionPipeline = true;
      #   modesetting.enable = true;
      #   powerManagement.enable = false;
      #   powerManagement.finegrained = false;
      #   open = false;
      #   nvidiaSettings = true;
      #   # https://github.com/NixOS/nixpkgs/blob/nixos-unstable/pkgs/os-specific/linux/nvidia-x11/default.nix
      #   package = config.boot.kernelPackages.nvidiaPackages.production;
      #   # package = config.boot.kernelPackages.nvidiaPackages.
      #   # pf
      #   # nvidiaPersistenced = true;
      #   prime = {
      #     #   # sync.enable = true;
      #     #   reverseSync.enable = true;
      #     #   allowExternalGpu = true;

      #     # amdgpuBusId = "PCI:193:0:0";
      #     # nvidiaBusId = "PCI:5:0:0";

      #     # amdgpuBusId = "PCI:193:0:0";
      #     # nvidiaBusId = "PCI:5:0:0";

      #     #   # offload = {
      #     #   #   enable = true;
      #     #   #   enableOffloadCmd = true;
      #     #   # };
      #   };

      # };
    };
  };
}
