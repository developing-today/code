# oc-password-sync — sync the OpenChamber upstream password from
# the prod OpenCode service file (developing-today/openchamber#894).
# Oneshot enabled into default.target, so it runs at user-session
# start. Script: ~/.local/bin/oc-password-sync.sh.
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with services.oc-password-sync.enable = true.
#
# Folded into the flake 2026-11-01 (developing-today/code#3720):
# until now the unit was a plain file in ~/.config/systemd/user.
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
  };
}
