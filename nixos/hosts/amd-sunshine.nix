# amd-sunshine.nix — Sunshine game-stream host + Moonlight client on `amd`,
# per the GUI-access verdict in developing-today/code#5924 (2026-11-04):
# Sunshine/Moonlight is the primary interactive-desktop path, RustDesk is
# demoted to greeter/fallback, Chrome Remote Desktop is break-glass only.
#
# Split from amd-krdp.nix on purpose: KRdp is bound to the Plasma session,
# while this host runs Hyprland (Wayland, wlroots) under SDDM on seat0.
# Sunshine captures through KMS directly (capture = "kms"), so it works
# under a wlroots compositor without depending on the desktop stack; the
# nixpkgs module runs it as a systemd USER service chained to
# graphical-session.target, i.e. it starts with the graphical login.
#
# Hardware path: the laptop GPU is a Radeon 780M iGPU (verified 2026-11-04:
# `vainfo` reports VAAPI H.264 + HEVC EncSlice profiles on the Mesa
# radeonsi driver), so the hardware encoder is VAAPI. There is no NVENC
# (no NVIDIA GPU) and no QuickSync (not Intel).
#
# THE NAMED FAILURE MODE (#5924): Sunshine's probe takes the configured
# encoder first, but if VAAPI fails validation it LOGS an error and then
# falls through to the remaining encoders — software (libx264) included —
# and the stream "works" on CPU while burning it. Verified in the pinned
# Sunshine source (src/video.cpp, probe_encoders): `encoder = "vaapi"`
# below is therefore a PREFERENCE, not a guarantee. The guarantee is the
# alarm: `sunshine-encoder-check` runs hourly (and 10 min after boot) as
# a user service and FAILS VISIBLY (the unit lands in
# `systemctl --user --failed`) when this boot's journal shows the pinned
# encoder was rejected ("Couldn't find any working encoder matching
# [vaapi]"), no encoder was found at all, or the probe/stream winner was
# software ("Found H.264 encoder: ... [software]" /
# "Creating encoder [software]"). It passes only on positive evidence of
# VAAPI: "Found H.264 encoder: h264_vaapi [vaapi]" (or the HEVC/AV1
# equivalents) or a stream-time "Creating encoder [vaapi]".
#
# Access posture: tailnet-only. openFirewall stays FALSE — the laptop's
# tailscale0 interface is trusted in the firewall (see
# nixos/abstract/tailscale-autoconnect), so Sunshine's ports are reachable
# from tailnet peers and are NOT opened to the LAN. Nothing here exposes
# Sunshine publicly, and CRD is untouched (break-glass only, per verdict).
#
# One-time setup (imperative state, deliberately not in this repo):
#   1. After the switch, open https://<host>:47990 (Sunshine web UI,
#      reachable over the tailnet) and create the admin credentials;
#      Sunshine stores them hashed under ~/.config/sunshine.
#   2. In Moonlight on the client, add the host by its tailnet address
#      (amd-6.raptor-pumpkinseed.ts.net / 100.99.113.103) and enter the
#      PIN Sunshine shows in its web UI to pair.
#   3. During the first real session, run `sunshine-encoder-check` and
#      confirm it reports the VAAPI encoder (the #5924 spike criterion).
{ pkgs, ... }:
let
  # Loud check for the #5924 named failure mode, matched against the log
  # lines the pinned Sunshine actually emits (src/video.cpp,
  # src/main.cpp — strings verified 2026-11-04, see header). Run manually
  # any time; also wired to a timer below so a silent software fallback
  # surfaces in `systemctl --user --failed` without anyone watching.
  sunshine-encoder-check = pkgs.writeShellScriptBin "sunshine-encoder-check" ''
    set -u
    journal="$(${pkgs.systemd}/bin/journalctl --user -u sunshine.service -b --no-pager -o cat 2>/dev/null || true)"
    lines="$(printf '%s\n' "$journal" | grep -i "encoder" || true)"
    if [ -z "$lines" ]; then
      echo "sunshine-encoder-check: sunshine.service has not run yet this boot — nothing to verdict."
      exit 0
    fi
    echo "--- encoder lines from sunshine.service (this boot, last 15) ---"
    printf '%s\n' "$lines" | tail -15
    echo "-----------------------------------------------------------------"
    if printf '%s\n' "$lines" | grep -qE "Couldn.t find any working encoder|Video failed to find working encoder|Unable to find display or encoder during startup"; then
      echo "FAIL: Sunshine found no working (hardware) encoder this boot. VAAPI is broken — check /dev/dri/renderD128 access, the video group, and Mesa/VAAPI drivers." >&2
      exit 1
    fi
    if printf '%s\n' "$lines" | grep -qE "Found (H\.264|HEVC|AV1) encoder:.*\[software\]|Creating encoder \[(software|libx264)\]"; then
      echo "FAIL: Sunshine is encoding in SOFTWARE (CPU) — the exact silent-fallback failure mode of code#5924. VAAPI must be fixed; do not treat streaming as healthy." >&2
      exit 1
    fi
    if ! printf '%s\n' "$lines" | grep -qE "Found (H\.264|HEVC|AV1) encoder:.*\[vaapi\]|Creating encoder \[vaapi\]"; then
      echo "sunshine-encoder-check: no VAAPI confirmation in this boot's log yet (no completed encoder probe?) — inconclusive, not a failure."
      exit 0
    fi
    echo "OK: VAAPI hardware encoder confirmed in Sunshine's log."
    exit 0
  '';
in
{
  services.sunshine = {
    enable = true;
    # CAP_SYS_ADMIN is what lets the KMS capture path grab the display
    # without running the whole daemon as root (module wraps the binary).
    capSysAdmin = true;
    # FALSE on purpose: tailscale0 is a trusted interface on this host,
    # so Sunshine is reachable over the tailnet only. Setting this true
    # would also punch the ports open on the LAN for no benefit.
    openFirewall = false;
    autoStart = true;
    settings = {
      sunshine_name = "amd";
      # Grab frames from KMS/DRM directly — the capture path that works
      # under Hyprland/wlroots (the wlr portal path is the alternative;
      # KMS is the lower-latency, compositor-independent one).
      capture = "kms";
      # Preference pin toward VAAPI (see header: the probe can still
      # fall through to software if VAAPI fails validation — which is
      # why the encoder check below exists and runs hourly).
      encoder = "vaapi";
    };
  };

  home-manager.users.user = {
    home.packages = [
      # Moonlight-Qt: the client half of the verdict. This laptop is
      # both the stream host and a client (pair it with other hosts);
      # pairing state lives in ~/.local/share/Moonlight Game Streaming
      # Project/ and is created interactively — see header.
      pkgs.moonlight-qt
      sunshine-encoder-check
    ];

    systemd.user.services.sunshine-encoder-check = {
      Unit.Description = "Fail loudly if Sunshine fell back to a software encoder (code#5924)";
      Service = {
        Type = "oneshot";
        ExecStart = "${sunshine-encoder-check}/bin/sunshine-encoder-check";
      };
    };
    systemd.user.timers.sunshine-encoder-check = {
      Unit.Description = "Hourly Sunshine encoder check (code#5924)";
      Timer = {
        OnBootSec = "10min";
        OnUnitActiveSec = "1h";
        Unit = "sunshine-encoder-check.service";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
