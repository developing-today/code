# laptop-repo-keeper — hourly sweep keeping the laptop's git
# primaries committed, current and on their default branch
# (Drew, 2026-10-28; developing-today/code#1723).
#
# Home-manager module: imported by the amd host config
# (nixos/hosts/default.nix) with
# services.laptop-repo-keeper.enable = true. The keeper script is
# vendored beside this module (./laptop-repo-keeper.sh); its rules
# (what it commits, pulls, pushes and never touches) are documented
# in the script header.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.laptop-repo-keeper;
  home = config.home.homeDirectory;
  keeperPkg = pkgs.writeShellScriptBin "laptop-repo-keeper" (
    builtins.readFile ./laptop-repo-keeper.sh
  );
  servicePath = lib.makeBinPath [
    pkgs.git
    pkgs.coreutils
    pkgs.findutils
    pkgs.gnugrep
    pkgs.gawk
  ];
in
{
  options.services.laptop-repo-keeper = {
    enable = lib.mkEnableOption "the laptop repo keeper sweep";

    onCalendar = lib.mkOption {
      type = lib.types.str;
      default = "*-*-* *:23:00";
      description = "Hourly run time (local) for the repo keeper sweep.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.user.services.laptop-repo-keeper = {
      Unit.Description = "Laptop repo keeper: primaries committed, pulled, pushed, on default branch (code#1723)";
      Service = {
        Type = "oneshot";
        ExecStart = "${keeperPkg}/bin/laptop-repo-keeper";
        Environment = "PATH=${servicePath}:${home}/tools/bin:/run/current-system/sw/bin";
        TimeoutStartSec = "15m";
      };
    };

    systemd.user.timers.laptop-repo-keeper = {
      Unit.Description = "Run the laptop repo keeper hourly";
      Timer = {
        OnCalendar = cfg.onCalendar;
        Persistent = true;
        RandomizedDelaySec = "5m";
      };
      Install.WantedBy = [ "timers.target" ];
    };
  };
}
