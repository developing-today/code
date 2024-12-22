{ pkgs, config, ... }:
{
  boot = {
    kernelParams = [
      "acpi_rev_override"
      "mem_sleep_default=deep"
      "intel_iommu=igfx_off"
      "nvidia-drm.modeset=1"
    ];
    kernelPackages = pkgs.linuxPackages_5_4;
    extraModulePackages = [ config.boot.kernelPackages.nvidia_x11 ];
  };

  # Enable the X11 windowing system.
  services.xserver.enable = true;

  # Enable the KDE Desktop Environment.
  services.xserver.displayManager.sddm.enable = true;
  services.xserver.desktopManager.plasma5.enable = true;

  ## NVIDIA
  services.xserver = {
    videoDrivers = [ "nvidia" ];

    config = ''
      Section "Device"
          Identifier  "Intel Graphics"
          Driver      "intel"
          #Option      "AccelMethod"  "sna" # default
          #Option      "AccelMethod"  "uxa" # fallback
          Option      "TearFree"        "true"
          Option      "SwapbuffersWait" "true"
          BusID       "PCI:0:2:0"
          #Option      "DRI" "2"             # DRI3 is now default
      EndSection

      Section "Device"
          Identifier "nvidia"
          Driver "nvidia"
          BusID "PCI:1:0:0"
          Option "AllowEmptyInitialConfiguration"
      EndSection
    '';
    screenSection = ''
      Option         "metamodes" "nvidia-auto-select +0+0 {ForceFullCompositionPipeline=On}"
      Option         "AllowIndirectGLXProtocol" "off"
      Option         "TripleBuffer" "on"
    '';
  };

  hardware.nvidia.prime = {
    # Sync Mode
    sync.enable = true;
    # Offload Mode
    #offload.enable = true;

    # Bus ID of the NVIDIA GPU. You can find it using lspci, either under 3D or VGA
    nvidiaBusId = "PCI:1:0:0";

    # Bus ID of the Intel GPU. You can find it using lspci, either under 3D or VGA
    intelBusId = "PCI:0:2:0";
  };
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
