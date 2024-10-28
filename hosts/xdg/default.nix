{
  pkgs,
  ...
}: {
  xdg = {
    portal = {
      extraPortals = [ pkgs.xdg-desktop-portal-hyprland ];
      config.common.default = "gtk";
    };
  };
}
