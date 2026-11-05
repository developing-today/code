{ pkgs, ... }:
let
  # The script the rustdesk-password-assert service runs.
  rustdeskPasswordAssert = pkgs.writeShellScript "rustdesk-password-assert" ''
    set -eu

    # Fetch the password in the owning user context. Three traps,
    # all hit on 2026-11-05: the fleet session broker is bound to the
    # login session, so the durable route is the Bitwarden fallback,
    # which needs the access token exported from its on-disk file
    # (the same file the fleet-feed collector unit names via
    # BWS_ACCESS_TOKEN_FILE); runuser preserves the caller
    # environment, so HOME and XDG_CONFIG_HOME must be pinned or
    # the fleet CLI reads /root/.config/fleet and fails; and a root process cannot use
    # the user session broker at all.
    pw=""
    for _ in $(seq 1 60); do
      if pw="$(${pkgs.util-linux}/bin/runuser -u user -- env HOME=/home/user XDG_CONFIG_HOME=/home/user/.config sh -c 'export BWS_ACCESS_TOKEN="$(cat /home/user/.config/bws/access-token)"; exec /home/user/.local/bin/fleet secrets get rustdesk.amd-6-password' 2>/dev/null)" && [ -n "$pw" ]; then
        break
      fi
      pw=""
      sleep 5
    done
    if [ -z "$pw" ]; then
      echo "rustdesk-password-assert: could not fetch rustdesk.amd-6-password from the vault" >&2
      exit 1
    fi

    # The password is applied through the running user service IPC
    # socket; wait for the user session to bring it up.
    for _ in $(seq 1 60); do
      if [ -S /tmp/RustDesk-1337/ipc ]; then
        break
      fi
      sleep 5
    done

    # rustdesk --password refuses to run unprivileged and locates the
    # user service IPC socket via XDG_RUNTIME_DIR, so this service
    # runs as root with the user HOME and runtime dir pinned.
    env HOME=/home/user XDG_RUNTIME_DIR=/run/user/1337 \
      /etc/profiles/per-user/user/bin/rustdesk --password "$pw"
    unset pw
  '';
in
{
  # RustDesk password durability for the amd laptop (code#5694).
  #
  # RustDesk itself stays imperative on this host (the package is in
  # home/common, the user service is a hand-written unit; KRdp in
  # ./amd-krdp.nix is the declarative successor). Its permanent
  # password lives in ~/.config/rustdesk/RustDesk.toml, in the same
  # file as the ed25519 key pair the machine ID is derived from, so
  # any config regeneration (fresh key pair, new ID) silently drops
  # the password. Imperative config surgery by fleet lanes on
  # 2026-11-04 desynced the laptop from the vault copy (BWS secret
  # rustdesk.amd-6-password) the same way.
  #
  # This service makes the vault the source of truth: at every boot
  # it re-asserts the vault password onto whatever config the
  # RustDesk service currently has, so the password follows the
  # vault across machine-ID changes and config regeneration. The
  # password value never appears in this repo or the Nix store; the
  # script names the secret and resolves it at run time.
  #
  # Boot-only by design: applying a password restarts the RustDesk
  # server component, which is free at boot but would drop a live
  # session mid-flight, so there is deliberately no periodic timer.
  # After mid-session config surgery, re-run on demand with
  # `systemctl start rustdesk-password-assert`.
  #
  # Activation: takes effect at the next system switch that includes
  # this module. Until then the rotated password is already live on
  # the laptop; only the boot re-assertion is pending.
  systemd.services.rustdesk-password-assert = {
    description = "Re-assert the RustDesk permanent password from the vault";
    after = [ "network.target" ];
    wantedBy = [ "multi-user.target" ];
    # The fetch script's vault route is fleet secrets get's Bitwarden
    # fallback, which spawns `bws` (the broker leg is bound to the
    # login session by design). Without bws on the unit PATH the
    # first real activation (gen 537, 2026-11-05) failed all 60
    # retries with "bws secret list: spawn failed: entity not found".
    path = [ pkgs.bws ];
    serviceConfig = {
      Type = "oneshot";
      ExecStart = rustdeskPasswordAssert;
    };
  };
}
