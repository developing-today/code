{
  pkgs,
  ...
}: {
  xdg = {
    portal = {
      extraPortals = with pkgs; [
        xdg-desktop-portal-hyprland
        xdg-desktop-portal-gtk
        xdg-desktop-portal-kde
      ];
      config.common.default = "hyprland"; # "*";
    };
  };
}
