# oc-password-sync — sync the OpenChamber upstream password from
# the prod OpenCode service file (developing-today/openchamber#894).
# A .path unit watches the prod service file
# (~/.local/state/opencode/service-prod.json) and triggers the
# oneshot service on change; the .path unit is the one enabled into
# default.target. (The service file also carries its own
# [Install] WantedBy=default.target, dormant in practice; it is
# replicated as installed.) Script: ~/.local/bin/oc-password-sync.sh.
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with services.oc-password-sync.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until now the units were plain files in ~/.config/systemd/user.
# Semantics are replicated exactly; only provenance changes.
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
  };
}
