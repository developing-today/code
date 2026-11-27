# NixOS module for the `id` peer-to-peer file sharing service.
#
# Multi-instance module: each entry in `services.id.instances` creates an
# isolated systemd service with its own working directory.
#
# Usage in a NixOS configuration:
#   imports = [ ./nix/id-module.nix ];
#   services.id = {
#     package = idPackage;
#     instances.primary = {
#       enable = true;
#       web = true;
#       port = 3000;
#       ephemeral = true;
#     };
#     instances.secondary = {
#       enable = true;
#       web = true;
#       port = 3001;
#       ephemeral = true;
#     };
#   };
{ config, lib, ... }:
let
  cfg = config.services.id;

  instanceModule =
    { name, ... }:
    {
      options = {
        enable = lib.mkEnableOption "this id instance";

        web = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Enable the web interface.";
        };

        port = lib.mkOption {
          type = lib.types.port;
          default = 3000;
          description = "Port for the web interface.";
        };

        irohPort = lib.mkOption {
          type = lib.types.port;
          default = 0;
          description = "Port for the Iroh QUIC endpoint. 0 = random.";
        };

        ephemeral = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Use in-memory storage (content lost on restart).";
        };

        noRelay = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Disable relay servers, use direct connections only.";
        };

        noGossip = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Disable gossip-based peer discovery.";
        };

        noMdns = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Disable mDNS local peer discovery.";
        };

        world = {
          enable = lib.mkEnableOption "hosting multiplayer worlds (`id serve --world`)";

          host = lib.mkOption {
            type = lib.types.bool;
            default = false;
            description = ''
              Serve the worlds directory as a host-mode data root
              (`id serve --world-host`). The node then answers `id host`
              on `.id-worlds/host.sock` under its state directory, so root
              on the machine runs, for example,
              `id host list /var/lib/id-<name>/.id-worlds`.
              Needs `world.enable` and a persistent (non-ephemeral) instance.
            '';
          };

          idleSecs = lib.mkOption {
            type = lib.types.nullOr lib.types.ints.unsigned;
            default = null;
            description = ''
              Close worlds no session used for this many seconds
              (`--world-idle-secs`). `null` keeps the server's default:
              600, or 900 under `world.host`. `0` keeps worlds open and is
              refused under `world.host`.
            '';
          };
        };

        environmentFile = lib.mkOption {
          type = lib.types.nullOr lib.types.path;
          default = null;
          description = ''
            A systemd `EnvironmentFile` read when the service starts, kept
            out of the Nix store. With `world.enable` it must set
            `ID_WORLD_ADMIN_TOKEN`.
          '';
        };

        extraArgs = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          description = "Extra command-line arguments passed to `id serve`.";
        };

        openFirewall = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Open the web port in the firewall.";
        };
      };
    };

  # Filter to only enabled instances
  enabledInstances = lib.filterAttrs (_name: icfg: icfg.enable) cfg.instances;

  # Build a systemd service for a single instance
  mkService = name: icfg: {
    description = "id peer-to-peer file sharing service (${name})";
    wantedBy = [ "multi-user.target" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];

    serviceConfig = {
      ExecStart = lib.concatStringsSep " " (
        [
          "${cfg.package}/bin/id"
          "serve"
        ]
        ++ lib.optionals icfg.web [
          "--web"
          "--port"
          (builtins.toString icfg.port)
        ]
        ++ lib.optionals (icfg.irohPort != 0) [
          "--iroh-port"
          (builtins.toString icfg.irohPort)
        ]
        ++ lib.optional icfg.ephemeral "--ephemeral"
        ++ lib.optional icfg.noRelay "--no-relay"
        ++ lib.optional icfg.noGossip "--no-gossip"
        ++ lib.optional icfg.noMdns "--no-mdns"
        ++ lib.optional icfg.world.enable "--world"
        ++ lib.optional icfg.world.host "--world-host"
        ++ lib.optionals (icfg.world.idleSecs != null) [
          "--world-idle-secs"
          (builtins.toString icfg.world.idleSecs)
        ]
        ++ icfg.extraArgs
      );
      Restart = "on-failure";
      RestartSec = 5;
      EnvironmentFile = lib.mkIf (icfg.environmentFile != null) icfg.environmentFile;

      # Hardening
      DynamicUser = true;
      StateDirectory = lib.mkIf (!icfg.ephemeral) "id-${name}";
      RuntimeDirectory = lib.mkIf icfg.ephemeral "id-${name}";
      WorkingDirectory = if icfg.ephemeral then "/run/id-${name}" else "/var/lib/id-${name}";
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = true;
      NoNewPrivileges = true;
    };
  };

  # Collect firewall ports from instances with openFirewall + web enabled
  firewallPorts = lib.concatMap (
    entry: if entry.value.openFirewall && entry.value.web then [ entry.value.port ] else [ ]
  ) (lib.attrsToList enabledInstances);

in
{
  options.services.id = {
    package = lib.mkOption {
      type = lib.types.package;
      description = "The id package to use (shared by all instances).";
    };

    instances = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule instanceModule);
      default = { };
      description = "Set of id service instances to run.";
    };
  };

  config = lib.mkIf (enabledInstances != { }) {
    assertions = lib.concatLists (
      lib.mapAttrsToList (name: icfg: [
        {
          assertion = icfg.world.host -> icfg.world.enable;
          message = "services.id.instances.${name}.world.host needs world.enable.";
        }
        {
          assertion = icfg.world.host -> !icfg.ephemeral;
          message = "services.id.instances.${name}.world.host needs a persistent instance (ephemeral = false).";
        }
        {
          assertion = icfg.world.enable -> icfg.environmentFile != null;
          message = "services.id.instances.${name}.world.enable needs environmentFile setting ID_WORLD_ADMIN_TOKEN.";
        }
      ]) enabledInstances
    );

    systemd.services = lib.mapAttrs' (
      name: icfg: lib.nameValuePair "id-${name}" (mkService name icfg)
    ) enabledInstances;

    networking.firewall.allowedTCPPorts = lib.mkIf (firewallPorts != [ ]) firewallPorts;
  };
}
