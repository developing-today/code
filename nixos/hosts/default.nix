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
      "services/lab-tunnel"
      "services/pm-apps"
      "services/power-guard"
      "services/openai-cred-watchdog"
      "services/opencode-go-seating-guard"
      "services/opencode-watchdog"
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
        # Lab cluster Cloudflare Tunnel (code#6455): connector +
        # edge status service for lab.security.cab.
        services.lab-tunnel.enable = true;
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
        # OpenAI credential watchdog (code#3216): hourly user timer
        # asserting the OpenCode credential store never regains an
        # active token-sharing credential for the GPT providers.
        services.openai-cred-watchdog.enable = true;
      }
      {
        # OpenCode Go seating guard (code#5611): hourly user timer
        # asserting every Go leaf seated in a pool has an exact-id active
        # credential and has recorded a successful route attempt recently.
        services.opencode-go-seating-guard.enable = true;
      }
      {
        # OpenCode watchdog (code#7780): per-minute user timer asserting
        # opencode.service is active or port 13420 is serving; an
        # exit-0 non-serving restart sticks dark under Restart=on-failure,
        # so after a 120s dark grace window the watchdog alerts and
        # restarts the unit (Restart=always would race lifecycle
        # handovers — see the module).
        services.opencode-watchdog = {
          enable = true;
          # The build the watchdog's build-identity check (code#8255)
          # expects on the port, read from the name of the very
          # opencode package derivation the unit runs (openchamber's
          # nested opencode input — the same derivation home/common's
          # cascade wraps), so expectation and ExecStart resolve from
          # one derivation and cannot drift apart. This option lives
          # at the NixOS level (this module is a NixOS import); the
          # home-manager copy of this setting that #8255 landed in
          # home/common/default.nix named an option that does not
          # exist there and broke evaluation of the whole config.
          expectedVersion =
            let
              m = builtins.match "opencode-(.*)" inputs.openchamber.inputs.opencode.packages.x86_64-linux.opencode.name;
            in
            if m == null then "" else builtins.head m;
        };
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
      {
        # Laptop scratch sweep (darwin-nix#373): daily reaping of
        # opencode test/build scratch (>72h) and mcpx binary dirs
        # (>24h) - the two un-reaped scratch growth sources behind
        # the 2026-11-11 disk catch.
        home-manager.users.user = {
          imports = [ ../home/laptop-scratch-sweep.nix ];
          services.laptop-scratch-sweep.enable = true;
        };
      }
      {
        # GitHub backup mirror timers (code#3215): webhook-queue
        # drain every 5 min, daily mirror fetch, weekly reconcile.
        # Folded into the flake 2026-11-01 (code#3720); the scripts
        # run from the primary checkout (infra/github-backup).
        home-manager.users.user = {
          imports = [ ../home/github-backup.nix ];
          services.github-backup.enable = true;
        };
      }
      {
        # Worktree reaper (code#3698): hourly retirement of dead
        # fleet worktrees under the 6-hour quiet rule. Folded into
        # the flake 2026-11-01 (code#3720); ExecStart points at the
        # primary checkout, retiring the bootstrap-worktree install.
        home-manager.users.user = {
          imports = [ ../home/worktree-reaper.nix ];
          services.worktree-reaper.enable = true;
        };
      }
      {
        # Quota exporter (code#1048): Cloudflare quota snapshots
        # every 15 min for the quota gatherer. Folded into the
        # flake 2026-11-01 (code#3720). (The Meta exporter was
        # retired 2026-11-03: quota is gathered by asking OpenCode,
        # PR #5040.)
        home-manager.users.user = {
          imports = [
            ../home/cloudflare-quota-export.nix
          ];
          services.cloudflare-quota-export.enable = true;
        };
      }
      {
        # Dashboard backup, laptop leg (code#1459) — the older
        # ~/tools job, distinct from pm-dashboard-backup above.
        # Folded into the flake 2026-11-01 (code#3720).
        home-manager.users.user = {
          imports = [ ../home/dashboard-backup.nix ];
          services.dashboard-backup.enable = true;
        };
      }
      {
        # OpenChamber password sync (openchamber#894): a .path unit
        # watches the prod OpenCode service file and triggers the
        # sync oneshot when the password rotates.
        # Folded into the flake 2026-11-01 (code#3720).
        home-manager.users.user = {
          imports = [ ../home/oc-password-sync.nix ];
          services.oc-password-sync.enable = true;
        };
      }
      (
        { config, ... }:
        let
          loginKeyringRules = config.security.pam.services.login.rules;
        in
        {
          # GNOME Keyring unlock at graphical login (code#5477):
          # the login password must reach pam_gnome_keyring in
          # SDDM's own PAM stack, or the login keyring collection
          # stays locked for the whole session and non-desktop
          # contexts (SSH, agents, user services) stall reading
          # keyring-held secrets (the gh token) on a prompter they
          # can never answer. enableGnomeKeyring states the intent,
          # but in this tree sddm's rule set is a custom delegation
          # set (every phase includes/substacks login), into which
          # the flag injects nothing - verified at eval: the
          # generated pam.d/sddm is byte-identical with the flag on
          # or off. So the keyring rules are added to sddm's stack
          # explicitly, mirroring the login service's own keyring
          # rules; module path and order are referenced from the
          # login service, not copied, so they track nixpkgs.
          security.pam.services.sddm.enableGnomeKeyring = true;
          security.pam.services.sddm.rules = {
            auth.gnome_keyring = {
              enable = true;
              control = "optional";
              modulePath = loginKeyringRules.auth.gnome_keyring.modulePath;
              order = loginKeyringRules.auth.gnome_keyring.order;
            };
            password.gnome_keyring = {
              enable = true;
              control = "optional";
              modulePath = loginKeyringRules.password.gnome_keyring.modulePath;
              order = loginKeyringRules.password.gnome_keyring.order;
              settings.use_authtok = true;
            };
            session.gnome_keyring = {
              enable = true;
              control = "optional";
              modulePath = loginKeyringRules.session.gnome_keyring.modulePath;
              order = loginKeyringRules.session.gnome_keyring.order;
              settings.auto_start = true;
            };
          };
        }
      )
      # KRdp remote desktop for this laptop (user service + package);
      # see the file for the one-time on-machine credential setup.
      ./amd-krdp.nix
      # Sunshine + Moonlight primary remote desktop (code#5924):
      # VAAPI-pinned streaming host, Moonlight client, encoder
      # watchdog. KRdp above remains the Plasma-side option.
      ./amd-sunshine.nix
      # RustDesk permanent-password re-assertion from the vault
      # (code#5694); RustDesk itself stays imperative, see the file.
      ./amd-rustdesk.nix
      ./amd-ipv4-preference.nix
      ./amd-openclaw-secrets.nix
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
