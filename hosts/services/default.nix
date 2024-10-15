{
  pkgs,
  ...
}:
{
  services = {
    # desktop only
    tailscale.enable = true; # needed?
    printing.enable = true;
    pipewire = {
      enable = true;
      audio.enable = true;
      pulse.enable = true;
      wireplumber.enable = true;
      alsa = {
        enable = true;
        support32Bit = true;
      };
      jack.enable = true;
    };
    kanata.enable = true;
    flatpak.enable = true;
    dbus.enable = true;
    openssh = {
      enable = true;
      settings = {
        PermitRootLogin = "no";
        PasswordAuthentication = false;
      };
      hostKeys = [
        #   {
        #     path = "/etc/ssh/ssh_host_ed25519_key";
        #     type = "ed25519";
        #   }
        # ] ++ lib.optionals host.bootstrap
        # [
        {
          path = "/nix/persistent/etc/ssh/ssh_host_ed25519_key";
          type = "ed25519";
        }
      ];

        # extraConfig = ''
        #   Match user git
        #     AllowTcpForwarding no
        #     AllowAgentForwarding no
        #     PasswordAuthentication no
        #     PermitTTY no
        #     X11Forwarding no
        # '';
    };
    locate = {
      enable = true;
      package = pkgs.plocate;
      interval = "hourly";
      localuser = null;
    };
    displayManager = {
      #autoLogin = { enable = true; user = "user"; }; # security risk?
      defaultSession = "hyprland"; # for better or worse
      sddm.enable = true;
      #gdm.enable = true; # two at once bad
    };
    xserver = {
      enable = true;
      # libinput.enable = true;
      desktopManager = {
        # backup in case hyprland gets broken again
        #plasma6.enable = true; # bloat but kinda pretty
        #plasma5.enable = true; # bloat but kinda pretty
        gnome.enable = true;
      };
      xkb = {
        layout = "us";
        variant = "";
      };
    };
  };
}
