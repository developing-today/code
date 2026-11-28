{ config, ... }:
let
  dockerEnabled = config.virtualisation.docker.enable;
in
{
  virtualisation.podman = {
    enable = true;
    dockerCompat = !dockerEnabled;
    dockerSocket.enable = !dockerEnabled;
    defaultNetwork.settings.dns_enabled = true;
  };

  environment.persistence = {
    "/persist".directories = [ "/var/lib/containers" ];
  };
}
# { pkgs, ... }:
# {
#   virtualisation.podman = {
#     enable = true;

#     # Create a `docker` alias for podman, to use it as a drop-in replacement
#     dockerCompat = true;

#     # Required for containers under podman-compose to be able to talk to each other.
#     defaultNetwork.settings.dns_enabled = true;
#   };

#   environment.systemPackages = [ pkgs.podman-compose ];

#   environment.persistence = {
#     "/persist".directories = [ "/var/lib/containers" ];
#   };
# }
