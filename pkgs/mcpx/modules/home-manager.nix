# SPDX-License-Identifier: MIT OR Apache-2.0
#
# mcpx module — home-manager.
#
# Canonical home: developing-today/code, pkgs/mcpx/modules
# (code#4801). Same service semantics as the NixOS module
# (./nixos.nix, whose header carries the reasoning): start at login,
# keep running, a 20 s stop window, $HOME as the working directory,
# and the rendered config named with `--config`. On Linux the unit
# is a systemd user service; on darwin it is a launchd user agent
# with the equivalent fields (KeepAlive / RunAtLoad / ExitTimeOut).
# The standalone nix-darwin rendering is ./darwin.nix; note the two
# darwin renderings differ in one respect — this one points launchd
# straight at the store path, while darwin.nix keeps the fixed-path
# program copy a nix-darwin host needs for a stable plist (see its
# header).
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.mcpx;
in
{
  imports = [ ./common.nix ];

  options.services.mcpx.addPackageToProfile = lib.mkOption {
    type = lib.types.bool;
    default = true;
    description = ''
      Whether the module adds `package` to `home.packages`, putting
      `bin/mcpx` on the profile PATH. Turn it off when another
      package in the profile already provides `bin/mcpx` — the fork
      opencode package's `bin/mcpx` is a link to this very build, and
      home-manager's buildEnv still counts the package and the link
      as two providers of the same path (code#5027). The daemon is
      unaffected either way: the service definition runs `package`
      directly, never through the profile.
    '';
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        home.packages = lib.mkIf cfg.addPackageToProfile [ cfg.package ];

        # Point every other mcpx client at the same config the
        # daemon runs, two ways: MCPX_CONFIG in the session
        # environment (mcpx reads exactly that file instead of
        # searching), and the discoverable
        # ~/.config/mcpx/config.json, a link whose resolved target
        # is the very store path the daemon is started with — so a
        # client that discovers it computes the same daemon socket.
        home.sessionVariables.MCPX_CONFIG = cfg.configFile;
        xdg.configFile."mcpx/config.json".source = cfg.configFile;
      }

      (lib.mkIf pkgs.stdenv.hostPlatform.isLinux {
        systemd.user.services.mcpx = {
          Unit.Description = "mcpx daemon";
          Install.WantedBy = [ "default.target" ];
          Service = {
            ExecStart = "${cfg.package}/bin/mcpx daemon --config ${cfg.configFile}";
            # The daemon environment, as [Service] Environment=
            # entries through home-manager's typed list option.
            # List definitions merge in the module system, so other
            # contributors' entries concatenate with these; and
            # session variables never compete here — home-manager
            # delivers those to the user manager via environment.d,
            # not as per-service Environment= lines. (A service-level
            # `environment` attribute, by contrast, falls through
            # the unit type's freeform and is serialized as a literal
            # [environment] section — code#5088.)
            Environment = lib.mapAttrsToList (name: value: "${name}=${value}") cfg.environment;
            Restart = "always";
            TimeoutStopSec = 20;
            WorkingDirectory = "%h";
          }
          // lib.optionalAttrs (cfg.environmentFile != null) {
            EnvironmentFile = cfg.environmentFile;
          };
        };
      })

      (lib.mkIf pkgs.stdenv.hostPlatform.isDarwin {
        assertions = [
          {
            assertion = cfg.environmentFile == null;
            message = "services.mcpx.environmentFile has no launchd equivalent; on darwin, hand the daemon credentials through files its servers point at instead.";
          }
        ];
        launchd.agents.mcpx = {
          enable = true;
          config = {
            ProgramArguments = [
              "${cfg.package}/bin/mcpx"
              "daemon"
              "--config"
              "${cfg.configFile}"
            ];
            EnvironmentVariables = cfg.environment;
            KeepAlive = true;
            RunAtLoad = true;
            ExitTimeOut = 20;
            WorkingDirectory = config.home.homeDirectory;
          };
        };
      })
    ]
  );
}
