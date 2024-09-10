# mostly stolen from https://github.com/hercules-ci/nixflk/blob/template/modules/services/torrent/qbittorrent.nix
{
  config,
  lib,
  pkgs,
  ...
}:
with lib;
let
  cfg = config.services.qbittorrent;
  configDir = "${cfg.dataDir}/.config";
  openFilesLimit = 4096;
in
{
  options.services.qbittorrent = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Run qBittorrent headlessly as systemwide daemon
      '';
    };

    dataDir = mkOption {
      type = types.path;
      default = "/var/lib/qbittorrent";
      description = ''
        The directory where qBittorrent will create files.
      '';
    };

    user = mkOption {
      type = types.str;
      default = "qbittorrent";
      description = ''
        User account under which qBittorrent runs.
      '';
    };

    group = mkOption {
      type = types.str;
      default = "qbittorrent";
      description = ''
        Group under which qBittorrent runs.
      '';
    };

    port = mkOption {
      type = types.port;
      default = 8080;
      description = ''
        qBittorrent web UI port.
      '';
    };

    openFirewall = mkOption {
      type = types.bool;
      default = false;
      description = ''
        Open services.qBittorrent.port to the outside network.
      '';
    };

    openFilesLimit = mkOption {
      default = openFilesLimit;
      description = ''
        Number of files to allow qBittorrent to open.
      '';
    };
  };

  config = mkIf cfg.enable {
    environment.systemPackages = [ pkgs.qbittorrent-nox ];

    networking.firewall = mkIf cfg.openFirewall {
      allowedTCPPorts = [ cfg.port ];
      allowedUDPPorts = [ cfg.port ];
    };

    systemd.tmpfiles.rules = [ "d '${cfg.dataDir}' 0700 ${cfg.user} ${cfg.group} - -" ];

    systemd.services.qbittorrent = {
      after = [ "network.target" ];
      description = "qBittorrent Daemon";
      wantedBy = [ "multi-user.target" ];
      path = [ pkgs.qbittorrent ];
      serviceConfig = {
        ExecStart = ''
          ${pkgs.qbittorrent-nox}/bin/qbittorrent-nox \
            --profile=${configDir} \
            --webui-port=${toString cfg.port}
        '';
        # To prevent "Quit & shutdown daemon" from working; we want systemd to
        # manage it!
        Restart = "on-success";
        User = cfg.user;
        Group = cfg.group;
        UMask = "0002";
        LimitNOFILE = cfg.openFilesLimit;
      };
    };

    users.users = mkIf (cfg.user == "qbittorrent") {
      qbittorrent = {
        inherit (cfg) group;
        home = cfg.dataDir;
        isSystemUser = true;
        description = "qBittorrent Daemon user";
      };
    };

    users.groups = mkIf (cfg.group == "qbittorrent") {
      qbittorrent = {
        gid = null;
      };
    };
  };
}
# let
#   uid = 9876;
#   gid = 9877;
# in
# {
#   virtualisation.oci-containers.containers.qbittorrent = {
#     image = "dyonr/qbittorrentvpn";
#     ports = [ "127.0.0.1:8081:8080" ];
#     extraOptions = [ "--cap-add=NET_ADMIN" ];
#     volumes = [
#       "/etc/wireguard/:/config/wireguard/"
#       "/var/lib/qbittorrent:/config"
#       "/shared/downloads:/downloads"
#     ];
#     environment = {
#       VPN_ENABLED = "yes";
#       VPN_TYPE = "wireguard";
#       LAN_NETWORK = "192.168.1.0/24";
#       ENABLE_SSL = "no";
#       PUID = toString uid;
#       PGID = toString gid;
#       RESTART_CONTAINER = "no";
#     };
#   };

#   services.nginx.virtualHosts."torrent.bizel.fr" = {
#     useACMEHost = "bizel.fr";
#     forceSSL = true;
#     locations."^~ /" = {
#       proxyPass = "http://localhost:8081";
#     };
#   };

#   environment.persistence."/persist" = {
#     directories = [
#       {
#         directory = "/var/lib/qbittorrent";
#         user = "qbittorrent";
#         group = "qbittorrent";
#         mode = "0750";
#       }
#       {
#         directory = "/etc/wireguard";
#         user = "root";
#         group = "root";
#         mode = "0750";
#       }
#     ];
#   };

#   users = {
#     users.qbittorrent = {
#       inherit uid;
#       group = "qbittorrent";
#       isSystemUser = true;
#     };
#     groups.qbittorrent = {
#       inherit gid;
#     };
#   };
# }
