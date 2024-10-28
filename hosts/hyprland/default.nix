{ inputs, pkgs, ... }:
{
  programs.hyprland = {
    enable = true;
    package = inputs.hyprland.packages.${pkgs.stdenv.hostPlatform.system}.hyprland;
    portalPackage = inputs.hyprland.packages.${pkgs.stdenv.hostPlatform.system}.xdg-desktop-portal-hyprland;

    xwayland = {
      enable = true;
      # hidpi = true;
    };

    # {pkgs, inputs, ...}: let
    #   pkgs-unstable = inputs.hyprland.inputs.nixpkgs.legacyPackages.${pkgs.stdenv.hostPlatform.system};
    # in {
    #   hardware.opengl = {
    #     package = pkgs-unstable.mesa.drivers;

    #     # if you also want 32-bit support (e.g for Steam)
    #     driSupport32Bit = true;
    #     package32 = pkgs-unstable.pkgsi686Linux.mesa.drivers;
    #   };
    # }
  };
  # xdg = {
  #   portal = {
  #     enable = true;
  #     extraPortals = [ pkgs.xdg-desktop-portal-kde ];
  #   };
  # };
  # environment.systemPackages = [
  #   pkgs.xdg-utils # xdg-open
  #   pkgs.qt5.qtwayland
  #   pkgs.qt6.qtwayland
  # ];
  # # Mostly from <https://www.reddit.com/r/NixOS/comments/137j18j/comment/ju6h25k/>
  # environment.sessionVariables =
  #   {
  #     NIXOS_OZONE_WL = "1";
  #     SDL_VIDEODRIVER = "wayland";
  #     _JAVA_AWT_WM_NONREPARENTING = "1";
  #     CLUTTER_BACKEND = "wayland";
  #     WLR_RENDERER = "vulkan";
  #   }
  #   // lib.mkIf (config.hardware.nvidia.package != null) {
  #     LIBVA_DRIVER_NAME = "nvidia";
  #     GBM_BACKEND = "nvidia";
  #     __GLX_VENDOR_LIBRARY_NAME = "nvidia";
  #     NVD_BACKEND = "direct";
  #   };
}
# nixpkgs.overlays = [
#   (final: prev:
#     {
#       hyprland = inputs.hyprland.packages.${pkgs.system}.hyprland;
#       wlroots-hyprland = inputs.hyprland.packages.${pkgs.system}.wlroots-hyprland;
#       wlroots = inputs.nixpkgs_unstable.legacyPackages.${pkgs.system}.wlroots;
#     })
#   (final: prev: {
#     wlroots = prev.wlroots.override {
#       xwayland = prev.xwayland;
#       mesa = pkgs.mesa;
#     };
#   })
#   (final: prev: {
#     wlroots = prev.wlroots.overrideAttrs (old: {
#       nativeBuildInputs = old.nativeBuildInputs ++
#         [ inputs.nixpkgs_unstable.legacyPackages.${pkgs.system}.libdrm ];
#     });
#   })
#   (final: prev: {
#     wlroots-hyprland = prev.wlroots-hyprland.override { wlroots = prev.wlroots; };
#   })
#   (final: prev: {
#     hyprland = prev.hyprland.override {
#       mesa = pkgs.mesa;
#       wlroots = prev.wlroots-hyprland;
#     };
#   })
