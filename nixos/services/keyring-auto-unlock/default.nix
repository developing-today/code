# keyring-auto-unlock — re-unlock gnome-keyring after daemon restarts (code#10724).
#
# Problem: the login keyring unlocks via PAM at graphical login, but a
# gnome-keyring-daemon restart outside a login session (e.g. the
# 2026-11-16 ~05:38 recovery restart) leaves it LOCKED until a human
# unlocks it at the screen — blocking every Secret Service consumer
# (agy's login lives there) with no prompter to answer.
#
# Mechanism (no password ever touches disk):
# 1. At graphical login, pam_exec (expose_authtok) in the sddm/login
#    PAM stacks pipes the typed password into keyring-password-stash,
#    which keeps it in the kernel user keyring (RAM-only, UID-scoped;
#    reboot clears it, the next login re-stashes it).
# 2. A systemd user .path unit watches the daemon control socket and
#    runs keyring-auto-unlock (oneshot) on every (re)start. If the
#    default collection is locked and a cached password exists, the
#    script feeds it to `gnome-keyring-daemon --unlock` and retries
#    while the daemon initializes.
#
# Degradation is always to today's behavior: no cache (no login since
# boot) or a stale cache (password changed) leaves the keyring locked,
# exactly as now; the next graphical login refreshes the cache. The
# legacy capital-`Login` collection (pre-existing, locked, empty
# labels) is intentionally untouched — the default `login` keyring is
# what PAM and agy use.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.keyring-auto-unlock;

  # This nixpkgs pin predates writers.writeShellApplication, so wrap
  # writeBashBin outputs with makeWrapper for runtime PATH. (The
  # sources are shellcheck-clean; re-check after edits with:
  # shellcheck -S warning nixos/services/keyring-auto-unlock/*.sh)
  mkWrappedBin =
    name: src: deps:
    pkgs.runCommand "${name}-wrapped" { nativeBuildInputs = [ pkgs.makeWrapper ]; } ''
      mkdir -p $out/bin
      makeWrapper ${pkgs.writers.writeBashBin name (builtins.readFile src)}/bin/${name} \
        $out/bin/${name} \
        --prefix PATH : ${lib.makeBinPath deps}
    '';

  stashPkg = mkWrappedBin "keyring-password-stash" ./keyring-password-stash.sh [
    pkgs.keyutils
    pkgs.util-linux
  ];

  unlockPkg = mkWrappedBin "keyring-auto-unlock" ./keyring-auto-unlock.sh [
    pkgs.keyutils
    pkgs.systemd
    pkgs.gnome-keyring
  ];

  # pam_exec invocation: flags first, store-path command last (the
  # command is everything after the last recognized flag). `optional`
  # so a stash failure can never fail a login; the stash script itself
  # also exits 0 unconditionally for the same reason.
  stashRule = {
    enable = true;
    control = "optional";
    modulePath = "${pkgs.linux-pam}/lib/security/pam_exec.so";
    order = cfg.pamOrder;
    args = [
      "expose_authtok"
      "quiet"
      "${stashPkg}/bin/keyring-password-stash"
    ];
  };
in
{
  options.services.keyring-auto-unlock = {
    enable = lib.mkEnableOption "re-unlock of the GNOME login keyring after out-of-session daemon restarts (code#10724)";

    user = lib.mkOption {
      type = lib.types.str;
      default = "user";
      description = "The user whose login password is cached and whose keyring is re-unlocked.";
    };

    pamOrder = lib.mkOption {
      type = lib.types.int;
      default = 12400;
      description = "PAM auth order for the stash rule: after pam_unix/keyring auth (so authtok exists), before final unix sufficient.";
    };
  };

  config = lib.mkIf cfg.enable {
    security.pam.services.sddm.rules.auth.keyring_stash = stashRule;
    security.pam.services.login.rules.auth.keyring_stash = stashRule;

    home-manager.users.${cfg.user} = {
      # keyctl on the user PATH: the test script and manual cache
      # inspection use it directly (the packaged scripts carry their
      # own wrapped PATH and do not rely on this).
      home.packages = [ pkgs.keyutils ];

      systemd.user.services.keyring-auto-unlock = {
        Unit.Description = "Re-unlock GNOME login keyring from login-cached password after daemon restart (code#10724)";
        Service = {
          Type = "oneshot";
          ExecStart = "${unlockPkg}/bin/keyring-auto-unlock";
        };
      };

      systemd.user.paths.keyring-auto-unlock = {
        Unit.Description = "Watch gnome-keyring-daemon control socket for restarts (code#10724)";
        Path = {
          PathChanged = "%t/keyring/control";
          Unit = "keyring-auto-unlock.service";
        };
        Install.WantedBy = [ "default.target" ];
      };
    };
  };
}
