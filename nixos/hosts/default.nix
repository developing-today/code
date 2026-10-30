inputs:
let
  host = inputs.self.lib.nixos-host-configuration;
in
{
  nixos = host {
    profiles = "desktop";
    hardware = "framework/13-inch/12th-gen-intel";
    disks = [
      "nvme0n1/2t"
      "tmpfs/root"
    ];
    users = [
      "user"
      "backup"
      "root"
    ];
    wireless = "us-wi-1";
  };
  amd = host {
    profiles = [
      "desktop"
      # "printing"
      "services/flatpak"
      "services/id-mail"
      "services/pm-stack"
      "services/pm-apps"
      "services/power-guard"
      # "microvm"
      # "server"
      "networking/dhcp-nat"
      "specialisations/nvidia-egpu"
    ];
    hardware = "framework/13-inch/7040-amd";
    disks = [
      "nvme0n1/4t"
      "tmpfs/root"
    ];
    users = [
      "user"
      "backup"
      "muse"
      "root"
    ];
    wireless = [
      "us-wi-1"
      "us-mn-1"
      "global-mobile-1"
      "us-wi-2"
      "us-global-1"
      "us-global-2"
    ];
    modules = [
      {
        services.id-mail = {
          transport = "worker";
          workerUrl = "https://mail.security.cab";
        };
      }
      {
        # PM dashboard stack (aggregator + projector, git-sync and
        # quota-gatherer timers) as NixOS-managed user services.
        services.pm-stack.enable = true;
      }
      {
        # PM app servers (code#1752): Kaneo + PostgreSQL, Grist, and
        # Pad as oci-containers, ready-for-agent as a user service —
        # the last hand-run servers, now flake-declared.
        services.pm-apps.enable = true;
        # Power guard (code#1687): confirm critical battery readings
        # before UPower can hybrid-sleep the machine.
        services.power-guard.enable = true;
      }
      {
        # Laptop backups (code#1516, dezren39/nix#976): the dashboard
        # backup (local + R2 + Drive) and the repo-archive restic job,
        # declared in this flake (nixos/home/laptop-backups.nix) as a
        # home-manager module so they deploy with the same switch.
        # (Moved out of dezren39/nix 2026-10-28 — that is Drew's
        # Darwin flake; laptop services live in this flake.)
        home-manager.users.user = {
          imports = [ ../home/laptop-backups.nix ];
          services.laptop-backups.enable = true;
        };
      }
      {
        # Codex remote control (code#1685, dezren39/nix#1028): the
        # Codex app-server daemon with remote control enabled, declared
        # in this flake (nixos/home/codex-remote-control.nix) as a
        # home-manager module so it deploys with the same switch,
        # like the laptop backups above. (Moved out of dezren39/nix
        # 2026-10-28, with the backups.)
        home-manager.users.user = {
          imports = [ ../home/codex-remote-control.nix ];
          services.codex-remote-control.enable = true;
        };
      }
      {
        # Laptop repo keeper (code#1723): hourly sweep keeping every
        # repo primary on this machine committed, pulled, pushed and
        # on its default branch.
        home-manager.users.user = {
          imports = [ ../home/laptop-repo-keeper.nix ];
          services.laptop-repo-keeper.enable = true;
        };
      }
      {
        # Laptop autocommit (code#1723): every-minute tracked-file
        # sweeps for openclaw, hatch and hatch-personal, plus the
        # daily untracked-file review.
        home-manager.users.user = {
          imports = [ ../home/laptop-autocommit.nix ];
          services.laptop-autocommit.enable = true;
        };
      }
      # KRdp remote desktop for this laptop (user service + package);
      # see the file for the one-time on-machine credential setup.
      ./amd-krdp.nix
      ./amd-ipv4-preference.nix
    ];
  };
  # amd-server = host {
  #   # profiles = [ "server" ];
  #   profiles = [ "all" ];
  #   # hardware = "generic/amd";
  #   # disks = [
  #   #   # "nvme0n1/1t"
  #   #   # "tmpfs/root"
  #   # ];
  #   users = [ "user-for-vm" ];
  #   # wireless = "us-wi-1";
  # };
}
# apu2c3 = host {
#   profiles = ["server"];
#   hardware = "pcengines/apu";
#   disks = [
#     "sda1/small"
#     "tmpfs/root"
#   ];
#   users = [ "server" ];
# };
# apu2c4
# apu2c4-with-wifi
# apu2c3-with-modem
# apu2c3-with-wifi
# apu2c3-with-modem-and-wifi
# pi0
# pi2
# pi3
# pi4
# pi5
# fire3
# amd-server
# intel-server
# generic profile to connect/mount data disks
# generic script/process to apply a different config and use that for auto-upgrade going forward
