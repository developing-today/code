# NixOS module for the `id` peer-to-peer file sharing service.
#
# Usage in a NixOS configuration:
#   imports = [ ./nix/id-module.nix ];
#   services.id = {
#     enable = true;
#     web = true;
#     port = 3000;
#     ephemeral = true;
#   };
{ config, lib, ... }:
let
  cfg = config.services.id;
in
{
  options.services.id = {
    enable = lib.mkEnableOption "id peer-to-peer file sharing service";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The id package to use.";
    };

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

  config = lib.mkIf cfg.enable {
    systemd.services.id = {
      description = "id peer-to-peer file sharing service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        ExecStart = lib.concatStringsSep " " (
          [
            "${cfg.package}/bin/id"
            "serve"
          ]
          ++ lib.optionals cfg.web [
            "--web"
            "--port"
            (builtins.toString cfg.port)
          ]
          ++ lib.optionals (cfg.irohPort != 0) [
            "--iroh-port"
            (builtins.toString cfg.irohPort)
          ]
          ++ lib.optional cfg.ephemeral "--ephemeral"
          ++ lib.optional cfg.noRelay "--no-relay"
          ++ lib.optional cfg.noGossip "--no-gossip"
          ++ lib.optional cfg.noMdns "--no-mdns"
          ++ cfg.extraArgs
        );
        Restart = "on-failure";
        RestartSec = 5;

        # Hardening
        DynamicUser = true;
        StateDirectory = lib.mkIf (!cfg.ephemeral) "id";
        WorkingDirectory = lib.mkIf (!cfg.ephemeral) "/var/lib/id";
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        NoNewPrivileges = true;
      };
    };

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
  };
}
