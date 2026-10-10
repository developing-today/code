{
  config,
  lib,
  pkgs,
  ...
}:
with lib;
let
  cfg = config.services.tailscaleAutoconnect;
in
{
  options.services.tailscaleAutoconnect = {
    enable = mkEnableOption "tailscaleAutoconnect";
    authkeyFile = mkOption {
      type = types.str;
      description = "The authkey to use for authentication with Tailscale";
    };
    loginServer = mkOption {
      type = types.str;
      default = "";
      description = "The login server to use for authentication with Tailscale";
    };
    advertiseExitNode = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to advertise this node as an exit node";
    };
    exitNode = mkOption {
      type = types.str;
      default = "";
      description = "The exit node to use for this node";
    };
    exitNodeAllowLanAccess = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to allow LAN access to this node";
    };
    acceptRoutes = mkOption {
      type = types.bool;
      default = false;
      description = "Whether to accept routes from other nodes";
    };
    acceptDns = mkOption {
      type = types.bool;
      default = true;
      description = ''
        Whether tailscaled may manage the system DNS configuration (MagicDNS).
        With dhcpcd + openresolv (no systemd-resolved) tailscaled takes over
        /etc/resolv.conf and only forwards to whatever upstreams it scraped from
        the old file. When a DHCP/Wi-Fi renewal momentarily empties that list it
        answers SERVFAIL ("no upstream resolvers set") for everything, sometimes
        permanently. Set to false to leave DNS alone.
      '';
    };
  };
  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.authkeyFile != "";
        message = "authkeyFile must be set";
      }
      {
        assertion = cfg.exitNodeAllowLanAccess -> cfg.exitNode != "";
        message = "exitNodeAllowLanAccess must be false if exitNode is not set";
      }
      {
        assertion = cfg.advertiseExitNode -> cfg.exitNode == "";
        message = "advertiseExitNode must be false if exitNode is set";
      }
      {
        assertion = cfg.acceptDns -> config.services.resolved.enable;
        message = "services.tailscaleAutoconnect.acceptDns requires services.resolved.enable (tailscaled + openresolv loses DNS when its upstream list empties)";
      }
    ];
    # Login/auth is handled by the NixOS built-in `tailscaled-autoconnect`
    # (services.tailscale.authKeyFile): it runs `tailscale up --auth-key ...` only
    # when the backend is NeedsLogin/NeedsMachineAuth. A previous hand-rolled
    # `tailscale-autoconnect` unit duplicated that, and failed on boot whenever the
    # network was not up yet (`tailscale up` blocks until connected -> timeout 124).
    # Don't let the built-in unit block boot for the default 90s while waiting for a
    # backend that cannot leave NoState without a network.
    systemd.services.tailscaled-autoconnect.serviceConfig.TimeoutStartSec = "20s";
    networking.firewall = {
      trustedInterfaces = [ "tailscale0" ];
      allowedUDPPorts = [ config.services.tailscale.port ];
    };
    services.tailscale = {
      enable = true;
      # Only used on first login (NeedsLogin). No empty-string entries: they would be
      # passed to `tailscale up` as bogus positional arguments.
      extraUpFlags =
        lib.optional (cfg.loginServer != "") "--login-server=${cfg.loginServer}"
        ++ lib.optional cfg.advertiseExitNode "--advertise-exit-node"
        ++ lib.optional (cfg.exitNode != "") "--exit-node=${cfg.exitNode}"
        ++ lib.optional cfg.exitNodeAllowLanAccess "--exit-node-allow-lan-access"
        ++ lib.optional cfg.acceptRoutes "--accept-routes"
        ++ lib.optional (!cfg.acceptDns) "--accept-dns=false";
      # Applied on every boot (`tailscaled-set`) so flags also reach an already
      # logged-in node whose persisted prefs predate the config. `tailscale set` does
      # not block waiting for connectivity. exit-node flags are only passed when set.
      extraSetFlags =
        [
          "--accept-routes=${lib.boolToString cfg.acceptRoutes}"
          "--accept-dns=${lib.boolToString cfg.acceptDns}"
          "--advertise-exit-node=${lib.boolToString cfg.advertiseExitNode}"
        ]
        ++ lib.optional (cfg.exitNode != "") "--exit-node=${cfg.exitNode}"
        ++ lib.optional cfg.exitNodeAllowLanAccess "--exit-node-allow-lan-access";
      authKeyFile = cfg.authkeyFile;
      useRoutingFeatures = if cfg.advertiseExitNode then "both" else "client"; # both or server?
      # services.tailscale.interfaceName = "userspace-networking";
      # networking.nftables.enable = true;
      # $ sudo tailscale cert ${MACHINE_NAME}.${TAILNET_NAME}
      # Enabling systemd-resolved https://nixos.wiki/wiki/Systemd-resolved
      # https://github.com/tailscale/tailscale/issues/4254
    };
  };
}
# {
#   imports = [ ../global/tailscale.nix ];
#   services.tailscale = {
#     useRoutingFeatures = "both";
#     extraUpFlags = [ "--advertise-exit-node" ];
#   };
# }
# { lib, ... }:
# {
#   services.tailscale = {
#     enable = true;
#     useRoutingFeatures = lib.mkDefault "client";
#     extraUpFlags = [ "--login-server https://tailscale.m7.rs" ];
#   };
#   networking.firewall.allowedUDPPorts = [ 41641 ]; # Facilitate firewall punching
#   environment.persistence = {
#     "/persist".directories = [ "/var/lib/tailscale" ];
#   };
# }
# {
#   config,
#   lib,
#   pkgs,
#   outputs,
#   ...
# }:
# {
#   imports = [ outputs.nixosModules.tailscale-autoconnect ];
#   services.tailscaleAutoconnect = {
#     enable = true;
#     authkeyFile = config.sops.secrets.tailscale_key.path;
#     loginServer = "https://headscale.ozeliurs.com";
#     advertiseExitNode = lib.mkDefault true;
#   };
#   sops.secrets.tailscale_key = {
#     restartUnits = [ "tailscale-autoconnect.service" ];
#     sopsFile = ../secrets.yaml;
#   };
#   environment.persistence = {
#     "/persist".directories = [ "/var/lib/tailscale" ];
#   };
# }
