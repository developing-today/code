{ pkgs, config, ... }:
{
  specialisation = {
    nvidia-egpu.configuration = {
      boot.initrd.kernelModules = [
        "nvidia"
        "i915"
        "nvidia_modeset"
        # "nvidia_uvm"
        "nvidia_drm"
      ];
      system.nixos.tags = [ "nvidia-egpu" ];
      hardware.graphics = {
        enable = true;
        enable32Bit = true;
        extraPackages = with pkgs; [ vaapiVdpau ];
      };
      # hardware.opengl = {
      #   enable = true;
      #   driSupport = true;
      #   driSupport32Bit = true;
      # };
      # nvidia-x11, nvidia-settings, and nvidia-persistenced.
      services.xserver.videoDrivers = [ "nvidia" ];
      hardware.nvidia = {
        #
        # export __NV_PRIME_RENDER_OFFLOAD=1
        # export __NV_PRIME_RENDER_OFFLOAD_PROVIDER=NVIDIA-G0
        # export __GLX_VENDOR_LIBRARY_NAME=nvidia
        # export __VK_LAYER_NV_optimus=NVIDIA_only
        # exec "$@"
        #               offload = {
        # 	enable = true;
        # 	enableOffloadCmd = true;
        # };
        modesetting.enable = true;
        powerManagement.enable = false;
        powerManagement.finegrained = false;
        open = false;
        nvidiaSettings = true;
        package = config.boot.kernelPackages.nvidiaPackages.latest;
        nvidiaPersistenced = true;
        prime = {
          # sync.enable = true;
          # reverseSync.enable = true;
          allowExternalGpu = true;
          amdgpuBusId = "PCI:193:0:0";
          nvidiaBusId = "PCI:12:0:0";

          # offload = {
          #   enable = true;
          #   enableOffloadCmd = true;
          # };
        };

      };
    };
  };
}
