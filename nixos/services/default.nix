{ pkgs, inputs, ... }:
{
  # solaar module was upstreamed into nixpkgs (programs.solaar); flake input removed
  services = {
    # desktop only
    gvfs.enable = true;

    # Removable-media mounting (needed by gvfs/MTP phone mounting).
    # Mirrors modules/shared.nix, which is only imported by the clan machine
    # path (machines/user/configuration.nix) and so does not reach this host.
    # NB: shared.nix writes `programs.gvfs.enable`; the real option is
    # `services.gvfs.enable`, set above.
    udisks2.enable = true;

    # mDNS / service discovery on the local network.
    avahi = {
      enable = true;
      nssmdns4 = true; # resolve .local hostnames
      openFirewall = true;
    };

    # solaar: see programs.solaar below (upstreamed to nixpkgs)
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
        # Agent fleets connect in bursts over slow relays (Tailscale DERP);
        # stock MaxStartups drops those handshakes. Raised 2026-10-25,
        # raised again 2026-10-28 (Drew) after nix#1020: bursts through
        # the tailnet still tripped the 100 cap during fleet-wide runs.
        MaxStartups = "200:30:500";
        # 50 -> 100 (2026-10-28): every fleet command multiplexes as a
        # channel over one shared connection; bursts of sweeps/queue
        # workers plus long-lived --watch commands can exhaust 50.
        MaxSessions = 100;
        # Unauthenticated slots recycle in half the stock time, so a
        # burst drains faster instead of piling up against MaxStartups.
        LoginGraceTime = 60;
        # Probe idle clients every 30s and drop ones that never answer:
        # dead DERP/tailnet connections stop holding sessions open,
        # live-but-quiet sessions survive 10 missed probes (~5 min).
        ClientAliveInterval = 30;
        ClientAliveCountMax = 10;
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
    desktopManager = {
      gnome = {
        # backup in case hyprland gets broken again
        #plasma6.enable = true; # bloat but kinda pretty
        #plasma5.enable = true; # bloat but kinda pretty
        enable = true;
      };
    };
    xserver = {
      enable = true;
      # libinput.enable = true;
      # displayManager.gdm.enable = true;
      xkb = {
        layout = "us";
        variant = "";
      };
    };
  };
  programs.solaar = {
    enable = true;
    userService = {
      enable = true;
      window = "hide"; # Show the window on startup (show, *hide*, only)
      batteryIcons = "regular"; # Which battery icons to use (*regular*, symbolic, solaar)
      extraArgs = [ ]; # Extra arguments to pass to solaar on startup (now a list)
    };
  };
}
