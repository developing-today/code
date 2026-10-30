{ pkgs, ... }:
{
  # KRdp -- KDE's native RDP server -- for the amd laptop only.
  #
  # This is the declarative counterpart to the imperative RustDesk setup:
  # RustDesk is the interim remote desktop, KRdp is the one that survives
  # rebuilds because it lives here. nixpkgs (pinned) has no NixOS or
  # home-manager module for KRdp -- kdePackages.krdp is only an optional
  # package of the Plasma 6 desktop module -- so the package and its user
  # service are wired by hand below, for user "user" only, via this
  # amd-only host module (see nixos/hosts/default.nix).
  #
  # The unit deliberately reuses the name and contents of the unit the
  # krdp package itself ships (share/systemd/user/app-org.kde.krdpserver
  # .service, preset-disabled upstream): the Remote Desktop page in System
  # Settings starts/stops/enables exactly this unit over D-Bus, so the
  # settings toggle and this declaration manage one and the same service
  # instead of fighting. WantedBy plasma-workspace.target means it starts
  # with the Plasma session and simply never runs in a non-Plasma one.
  #
  # No secrets in this repo, by design:
  # - RDP usernames/passwords are NOT passed on the command line (they
  #   would land in the world-readable Nix store and process list). They
  #   are managed on the machine in System Settings -> Networking ->
  #   Remote Desktop (the KCM ships in this same package) and are stored
  #   by KWallet, never in the repo.
  # - TLS: if no certificate is configured, krdpserver generates a
  #   temporary self-signed one; the KCM can also generate a persistent
  #   on-machine pair. Nothing certificate-like is committed either.
  #
  # One-time on-machine setup (after the switch that lands this):
  #   1. Open System Settings -> Networking -> Remote Desktop.
  #   2. Add the RDP user and password that clients will connect with.
  #   3. Leave the port at 3389. No firewall change is needed or wanted:
  #      the host firewall opens no TCP ports, but tailscale0 is a
  #      trusted interface (nixos/abstract/tailscale-autoconnect), so the
  #      server is reachable over Tailscale only -- never the LAN.
  # Until step 2 is done the service runs but has no user that can log
  # in, which is the safe default.
  home-manager.users.user = {
    home.packages = [ pkgs.kdePackages.krdp ];

    systemd.user.services."app-org.kde.krdpserver" = {
      Unit = {
        Description = "KRDP Server";
        After = [
          "plasma-xdg-desktop-portal-kde.service"
          "plasma-core.target"
        ];
      };
      Service = {
        Type = "exec";
        ExecStart = "${pkgs.kdePackages.krdp}/bin/krdpserver";
        Restart = "on-abnormal";
      };
      Install.WantedBy = [ "plasma-workspace.target" ];
    };
  };
}
