{ lib, ... }:
{
  services.tailscale = {
    enable = true;
    useRoutingFeatures = lib.mkDefault "client";
    extraUpFlags = [ "--login-server https://tailscale.m7.rs" ];
  };
  networking.firewall.allowedUDPPorts = [ 41641 ]; # Facilitate firewall punching

  environment.persistence = {
    "/persist".directories = [ "/var/lib/tailscale" ];
  };
}
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
