{ pkgs, ... }:
{
  xdg = {
    portal = {
      extraPortals = with pkgs; [
        xdg-desktop-portal-hyprland
        xdg-desktop-portal-gtk
        xdg-desktop-portal-kde
        xdg-desktop-portal-gnome
        xdg-desktop-portal-dde
      ];
      config.common.default = "hyprland"; # "*";
    };
  };
}
