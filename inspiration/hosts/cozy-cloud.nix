{
  lib,
  config,
  pkgs,
  ...
}:
with lib;
let
  cfg = config.services.cozy-cloud;
in
{
  options = {
    services.cozy-cloud = {
      enable = mkOption {
        default = false;
        type = types.bool;
        description = "Enable cozy service";
      };

      settings = {
        adminPasswordFile = mkOption {
          description = lib.mdDoc "Cozy admin password. Can be generated with `cozy-stack config passwd`";
          type = types.path;
        };

        host = mkOption {
          description = lib.mdDoc "Cozy host url.";
          default = "localhost";
          type = types.str;
        };

        port = mkOption {
          description = lib.mdDoc "Cozy host port.";
          default = 8080;
          type = types.int;
        };

        adminPort = mkOption {
          description = lib.mdDoc "Cozy host port for admin interface.";
          default = 6060;
          type = types.int;
        };

        encryptorKeyFile = mkOption {
          description = lib.mdDoc "Cozy encryption key path. Can be generated with `cozy-stack -c /dev/null config gen-keys <path>`";
          type = types.path;
        };

        decryptorKeyFile = mkOption {
          description = lib.mdDoc "Cozy decryption key path. Can be generated with `cozy-stack -c /dev/null config gen-keys <path>`";
          type = types.path;
        };

        couchdb = {
          host = mkOption {
            description = lib.mdDoc "Cozy couchdb host.";
            default = "localhost";
            type = types.str;
          };

          port = mkOption {
            description = lib.mdDoc "Cozy couchdb port.";
            default = 5984;
            type = types.port;
          };

          user = mkOption {
            description = lib.mdDoc "Cozy couchdb username.";
            default = "cozy";
            type = types.str;
          };

          pass = mkOption {
            description = lib.mdDoc "Cozy couchdb password.";
            default = "cozy";
            type = types.str;
          };
        };
      };
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = config.services.couchdb.enable;
        message = "CouchDB is required for Cozy to work.";
      }
    ];

    systemd.services.cozy-cloud = {
      description = "Cozy service";
      after = [
        "network.target"
        "couchdb.service"
      ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        User = "cozy";
        Group = "cozy";
        PermissionsStartOnly = true;
        WorkingDirectory = "/var/lib/cozy";
        ExecStart = "${pkgs.cozy-stack}/bin/cozy-stack serve";
        Restart = "always";
        # restricting the service. From <https://gitlab.archlinux.org/archlinux/packaging/packages/cozy-stack/-/blob/main/cozy-stack.service>
        AmbientCapabilities = "";
        CapabilityBoundingSet = "";
        LockPersonality = true;
        # Not compatible with NodeJS
        # MemoryDenyWriteExecute=true
        NoNewPrivileges = true;
        PrivateDevices = true;
        PrivateTmp = true;
        PrivateUsers = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectProc = "invisible";
        ProtectSystem = "full";
        RestrictAddressFamilies = "AF_INET AF_INET6 AF_NETLINK AF_UNIX";
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = "@system-service";
        SystemCallErrorNumber = "EPERM";
      };
    };

    users.users.cozy = {
      home = "/var/lib/cozy";
      group = "cozy";
      isSystemUser = true;
    };
    users.groups.cozy = { };

    environment.systemPackages = [ pkgs.cozy-stack ];

    systemd.tmpfiles.rules = [
      "Z /etc/cozy/cozy.json 0700 cozy cozy"
      "d /var/lib/cozy 0750 cozy cozy"
    ];

    environment.etc."cozy/cozy-admin-passphrase".source = cfg.settings.adminPasswordFile;
    environment.etc."cozy/cozy.json".text = builtins.toJSON {
      inherit (cfg.settings) host;
      inherit (cfg.settings) port;
      admin = {
        inherit (cfg.settings) host;
        port = cfg.settings.adminPort;
      };
      couchdb = {
        url = "http://${cfg.settings.couchdb.user}:${cfg.settings.couchdb.pass}@${cfg.settings.couchdb.host}:${toString cfg.settings.couchdb.port}/";
      };
      fs = {
        url = "file:///var/lib/cozy";
      };
      vault = {
        credentials_encryptor_key = cfg.settings.encryptorKeyFile;
        credentials_decryptor_key = cfg.settings.decryptorKeyFile;
      };
      konnectors = {
        cmd = "${pkgs.nodejs_18}/bin/node"; # fix node location
      };
      log = {
        level = "info";
        syslog = true;
      };
      jobs = {
        imagemagick_convert_cmd = "${lib.getExe pkgs.imagemagick}";
      };
      registries = {
        default = [
          "https://apps-registry.cozycloud.cc/selfhosted"
          "https://apps-registry.cozycloud.cc/mespapiers"
          "https://apps-registry.cozycloud.cc/banks"
          "https://apps-registry.cozycloud.cc/"
        ];
      };
    };
  };
}
# {
#   config,
#   outputs,
#   pkgs,
#   ...
# }:
# {
#   imports = [
#     outputs.nixosModules.couchdb-extended
#     outputs.nixosModules.cozy-cloud
#   ];

#   services.couchdb = {
#     enable = true;
#     adminPass = "pass";
#   };

#   services.couchdbExtended = {
#     ensureAdminUser = {
#       name = config.services.cozy-cloud.settings.couchdb.user;
#       passwordFile = pkgs.writeText "cozy-couchdb-password" config.services.cozy-cloud.settings.couchdb.pass;
#     };
#   };

#   services.cozy-cloud = {
#     enable = true;
#     settings = {
#       port = 8439;
#       adminPort = 8440;

#       decryptorKeyFile = config.sops.secrets.cozyDecKey.path;
#       encryptorKeyFile = config.sops.secrets.cozyEncKey.path;
#       adminPasswordFile = config.sops.secrets.cozyAdminPass.path;
#     };
#   };

#   services.nginx.virtualHosts."*.cozy.bizel.fr" = {
#     serverAliases = [ "cozy.bizel.fr" ];
#     useACMEHost = "bizel.fr";
#     forceSSL = true;
#     locations."^~ /" = {
#       proxyPass = "http://127.0.0.1:${toString config.services.cozy-cloud.settings.port}";
#       proxyWebsockets = true;
#     };
#   };

#   security.acme.certs."bizel.fr".extraDomainNames = [ "*.cozy.bizel.fr" ];

#   sops.secrets.cozyAdminPass = {
#     owner = "cozy";
#     sopsFile = ./secrets.yaml;
#   };
#   sops.secrets.cozyDecKey = {
#     owner = "cozy";
#     sopsFile = ./secrets.yaml;
#   };
#   sops.secrets.cozyEncKey = {
#     owner = "cozy";
#     sopsFile = ./secrets.yaml;
#   };
# }
