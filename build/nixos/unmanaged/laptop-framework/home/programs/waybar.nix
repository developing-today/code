{pkgs, ...}: {
  enable = true;
  systemd.enable = true;
  package = pkgs.waybar.overrideAttrs (oldAttrs: {
    mesonFlags = oldAttrs.mesonFlags ++ ["-Dexperimental=true"];
  });
  style = ''
    ${builtins.readFile "${pkgs.waybar}/etc/xdg/waybar/style.css"}

    window#waybar {
      background: transparent;
      border-bottom: none;
    }

    * {
        font-size: 18px;
    }
  '';
  settings = [
    {
      height = 30;
      layer = "top";
      position = "bottom";
      tray = {spacing = 10;};
      modules-center = ["hyprland/window"];
      modules-left = ["hyprland/workspaces" "hyprland/submap"];
      modules-right =
        [
          "pulseaudio"
          "network"
          "cpu"
          "memory"
          "temperature"
        ]
        ++ ["battery"]
        ++ [
          "clock"
          "tray"
        ];
      battery = {
        format = "{capacity}% {icon}";
        format-alt = "{time} {icon}";
        format-charging = "{capacity}% ";
        format-icons = ["" "" "" "" ""];
        format-plugged = "{capacity}% ";
        states = {
          critical = 15;
          warning = 30;
        };
      };
      clock = {
        format-alt = "{:%Y-%m-%d}";
        tooltip-format = "{:%Y-%m-%d | %H:%M}";
      };
      cpu = {
        format = "{usage}% ";
        tooltip = false;
      };
      memory = {format = "{}% ";};
      network = {
        interval = 1;
        format-alt = "{ifname}: {ipaddr}/{cidr}";
        format-disconnected = "Disconnected ⚠";
        format-ethernet = "{ifname}: {ipaddr}/{cidr}   up: {bandwidthUpBits} down: {bandwidthDownBits}";
        format-linked = "{ifname} (No IP) ";
        format-wifi = "{essid} ({signalStrength}%) ";
      };
      pulseaudio = {
        format = "{volume}% {icon} {format_source}";
        format-bluetooth = "{volume}% {icon} {format_source}";
        format-bluetooth-muted = " {icon} {format_source}";
        format-icons = {
          car = "";
          default = ["" "" ""];
          handsfree = "";
          headphones = "";
          headset = "";
          phone = "";
          portable = "";
        };
        format-muted = " {format_source}";
        format-source = "{volume}% ";
        format-source-muted = "";
        on-click = "pavucontrol";
      };
      "sway/mode" = {format = ''<span style="italic">{}</span>'';};
      temperature = {
        critical-threshold = 80;
        format = "{temperatureC}°C {icon}";
        format-icons = ["" "" ""];
      };
    }
  ];
}
