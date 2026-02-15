{ pkgs, ... }:
{
  enable = true;
  #   systemd.enable = true; # did not work?
  package = pkgs.waybar.overrideAttrs (oldAttrs: {
    mesonFlags = oldAttrs.mesonFlags ++ [ "-Dexperimental=true" ];
  });
  style = ''
    ${builtins.readFile "${pkgs.waybar}/etc/xdg/waybar/style.css"}

    window#waybar {
      background: transparent;
      border-bottom: none;
    }

    * {
        font-size: 12px;
        font-weight: bold;
        color: #00FF66;
        background-color: rgba(0, 0, 0, 0.1);
        padding: 0px;
        margin: 0px;
        min-height: 0px;
    }
  '';
  settings = [
    {
      start_hidden = true;
      height = 18; # the minimum effective number is affected by font-size and GTK_THEME
      layer = "top";
      position = "top"; # "bottom";
      tray = {
        spacing = 0;
      };
      modules-left = [
        "hyprland/workspaces"
        "hyprland/submap"
      ];
      modules-center = [ "hyprland/window" ];
      modules-right = [
        "pulseaudio"
        #           "network"
        "cpu"
        "memory"
        # "temperature"
      ]
      ++ [ "battery" ]
      ++ [
        "clock"
        "tray"
      ];
      battery = {
        format = "{capacity}%{icon}";
        format-alt = "{time}{icon}";
        format-charging = "{capacity}%";
        format-icons = [
          ""
          ""
          ""
          ""
          ""
        ];
        format-plugged = "{capacity}%";
        states = {
          critical = 15;
          warning = 30;
        };
      };
      clock = {
        format-alt = "{:%Y-%m-%d}";
        tooltip-format = "{:%Y-%m-%dT%H:%M}";
      };
      cpu = {
        format = "{usage}%";
        tooltip = false;
      };
      memory = {
        format = "{}%";
      };
      network = {
        interval = 1;
        format-alt = "{ifname}: {ipaddr}/{cidr}";
        format-disconnected = "Disconnected⚠";
        format-ethernet = "{ifname}: {ipaddr}/{cidr}   up: {bandwidthUpBits} down: {bandwidthDownBits}";
        format-linked = "{ifname} (No IP) ";
        format-wifi = "{essid} ({signalStrength}%) ";
      };
      pulseaudio = {
        format = "{volume}%{icon} {format_source}";
        format-bluetooth = "{volume}%{icon}{format_source}";
        format-bluetooth-muted = "{icon}{format_source}";
        format-icons = {
          car = "";
          default = [
            ""
            ""
            ""
          ];
          handsfree = "";
          headphones = "";
          headset = "";
          phone = "";
          portable = "";
        };
        format-muted = " {format_source}";
        format-source = "{volume}%";
        format-source-muted = "";
        on-click = "pavucontrol";
      };
      "hyprland/submap" = {
        format = ''<span style="italic">{}</span>'';
      };
      temperature = {
        critical-threshold = 80;
        format = "{temperatureC}°C {icon}";
        format-icons = [
          ""
          ""
          ""
        ];
      };
    }
  ];
}
