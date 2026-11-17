# oc-password-sync — sync the OpenChamber upstream password from
# the prod OpenCode service file (developing-today/openchamber#894).
# A .path unit watches the prod service file
# (~/.local/state/opencode/service-prod.json) and triggers the
# oneshot service on change; the .path unit is the one enabled into
# default.target. (The service file also carries its own
# [Install] WantedBy=default.target, dormant in practice; it is
# replicated as installed.) Script: ./oc-password-sync.sh, vendored
# beside this module and installed by home.file at the path
# ExecStart references (~/.local/bin/oc-password-sync.sh).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with services.oc-password-sync.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until now the units were plain files in ~/.config/systemd/user.
# Semantics are replicated exactly; only provenance changes.
#
# Script vendored 2026-11-01 (code#3724 reconciliation): the units
# were folded, but the script itself stayed an unmanaged operator
# file — and the corrected #3724 logic (drift judged against the
# running OpenChamber process's /proc environ; see the script's
# header) existed only there. The vendored copy is byte-identical
# to the verified live file at the time of the change
# (sha256 fc47b0d0ca4766a65a8a9225c865f59a920176a274e259adaa44d2f57d0a5df3),
# so home-manager taking over the path at the next activation
# changes no behavior: ExecStart and both units are untouched.
# (home-manager.backupFileExtension renames the operator file to
# *.backup on that activation; its bytes equal the store copy.)
{
  config,
  lib,
  ...
}:
let
  cfg = config.services.oc-password-sync;
  home = config.home.homeDirectory;
in
{
  options.services.oc-password-sync = {
    enable = lib.mkEnableOption "the OpenChamber upstream password sync (openchamber#894)";
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.oc-password-sync = {
      Unit.Description = "Sync OpenChamber upstream password from prod OpenCode service file (openchamber#894)";
      Service = {
        Type = "oneshot";
        ExecStart = "${home}/.local/bin/oc-password-sync.sh";
      };
      Install.WantedBy = [ "default.target" ];
    };

    systemd.user.paths.oc-password-sync = {
      Unit.Description = "Watch prod OpenCode service file for password rotation (openchamber#894)";
      Path = {
        PathChanged = "${home}/.local/state/opencode/service-prod.json";
        Unit = "oc-password-sync.service";
      };
      Install.WantedBy = [ "default.target" ];
    };

    # Provision the script at the exact path ExecStart references,
    # from the copy vendored beside this module (./oc-password-sync.sh).
    home.file.".local/bin/oc-password-sync.sh" = {
      source = ./oc-password-sync.sh;
      executable = true;
    };
  };
}
