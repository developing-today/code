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
    ];
    systemd.services.tailscale-autoconnect = {
      description = "Automatic connection to Tailscale";
      # make sure tailscale is running before trying to connect to tailscale
      after = [
        "network-pre.target"
        "tailscaled.service"
      ];
      wants = [
        "network-pre.target"
        "tailscaled.service"
      ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig.Type = "oneshot";
      script = with pkgs; ''
        set -x # -ex?
        # wait for tailscaled to settle
        sleep 2
        # check if we are already authenticated to tailscale
        status="$(${tailscale}/bin/tailscale status -json | ${jq}/bin/jq -r .BackendState)"
        # if status is not null, then we are already authenticated
        echo "tailscale status: $status"
        if [ "$status" != "NeedsLogin" ]; then
          ${coreutils}/bin/timeout 10 ${tailscale}/bin/tailscale up \
            ${lib.optionalString (cfg.loginServer != "") "--login-server=${cfg.loginServer}"} \
            ${lib.optionalString cfg.advertiseExitNode "--advertise-exit-node"} \
            ${lib.optionalString (cfg.exitNode != "") "--exit-node=${cfg.exitNode}"} \
            ${lib.optionalString cfg.exitNodeAllowLanAccess "--exit-node-allow-lan-access"} \
            ${lib.optionalString cfg.acceptRoutes "--accept-routes"}
          exit 0
        fi
        # otherwise authenticate with tailscale
        # timeout after 10 seconds to avoid hanging the boot process
        ${coreutils}/bin/timeout 10 ${tailscale}/bin/tailscale up \
          ${lib.optionalString (cfg.loginServer != "") "--login-server=${cfg.loginServer}"} \
          --authkey=$(cat "${cfg.authkeyFile}")
        # we have to proceed in two steps because some options are only available
        # after authentication
        ${coreutils}/bin/timeout 10 ${tailscale}/bin/tailscale up \
          ${lib.optionalString (cfg.loginServer != "") "--login-server=${cfg.loginServer}"} \
          ${lib.optionalString cfg.advertiseExitNode "--advertise-exit-node"} \
          ${lib.optionalString (cfg.exitNode != "") "--exit-node=${cfg.exitNode}"} \
          ${lib.optionalString cfg.exitNodeAllowLanAccess "--exit-node-allow-lan-access"} \
          ${lib.optionalString cfg.acceptRoutes "--accept-routes"}
      '';
    };
    networking.firewall = {
      trustedInterfaces = [ "tailscale0" ];
      allowedUDPPorts = [ config.services.tailscale.port ];
    };
    services.tailscale = {
      enable = true;
      extraUpFlags = [
        (lib.optionalString (cfg.loginServer != "") "--login-server=${cfg.loginServer}")
        (lib.optionalString cfg.advertiseExitNode "--advertise-exit-node")
        (lib.optionalString (cfg.exitNode != "") "--exit-node=${cfg.exitNode}")
        (lib.optionalString cfg.exitNodeAllowLanAccess "--exit-node-allow-lan-access")
        (lib.optionalString cfg.acceptRoutes "--accept-routes")
      ];
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
