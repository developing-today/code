{ pkgs, inputs, ... }:
{
  imports = [ inputs.solaar.nixosModules.default ];
  services = {
    # desktop only
    solaar = {
      enable = true; # Enable the service
      package = pkgs.solaar; # The package to use
      window = "hide"; # Show the window on startup (show, *hide*, only [window only])
      batteryIcons = "regular"; # Which battery icons to use (*regular*, symbolic, solaar)
      extraArgs = ""; # Extra arguments to pass to solaar on startup
    };
    tailscale.enable = true; # needed? # split this out or add to tailscale-autoconnect
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
    # https://nixos.org/manual/nixos/stable/index.html#module-services-flatpak
    # flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
    # flatpak update
    #
    dbus.enable = true;
    openssh = {
      # TODO: split-out openssh
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
      # authorized keys
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
      #localuser = null;
    };
    displayManager = {
      #autoLogin = { enable = true; user = "user"; }; # security risk?
      defaultSession = "hyprland"; # for better or worse
      sddm = {
        enable = true;
        wayland.enable = true;
        autoNumlock = true;
        enableHidpi = true;
      };
    };
    xserver = {
      enable = true;
      # libinput.enable = true;
      # displayManager.gdm.enable = true;
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
