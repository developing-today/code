{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  ...
}:
{
  imports = [
    inputs.impermanence.nixosModules.impermanence
    # inputs.nixosModules.home-manager.impermanence # home manager pulls this in?
  ];
  environment.persistence."/nix/persistent" = { # how to handle logical groups separate from key groups? should this be in hosts/environment/persistent and then imported here?
    hideMounts = true;

    directories = [
      "/home"
      "/root"
      "/var"
      "/tmp"
    ];
    files = [
      # "/etc/machine-id"
      "/etc/ssh/ssh_host_ed25519_key"
      # "/etc/ssh/ssh_host_ed25519_key.pub"
      # "/etc/ssh/ssh_host_rsa_key.pub"
      # "/etc/ssh/ssh_host_rsa_key"
    ];
  };
  systemd.services.nix-daemon = { # will this being here be an issue? should it be in hosts/nix-daemon/default.nix ? or hosts/systemd/default.nix ? or hosts/systemd/services/nix-daemon.nix ? or hosts/systemd/impermanence/default.nix ?
    environment = {
      # Location for temporary files
      TMPDIR = "/var/cache/nix";
    };
    serviceConfig = {
      # Create /var/cache/nix automatically on Nix Daemon start
      CacheDirectory = "nix";
    };
  };
}
# {
#   environment.persistence."/persistent" = {
#     enable = true;  # NB: Defaults to true, not needed
#     hideMounts = true;
#     directories = [
#       "/var/log"
#       "/var/lib/bluetooth"
#       "/var/lib/nixos"
#       "/var/lib/systemd/coredump"
#       "/etc/NetworkManager/system-connections"
#       { directory = "/var/lib/colord"; user = "colord"; group = "colord"; mode = "u=rwx,g=rx,o="; }
#     ];
#     files = [
#       "/etc/machine-id"
#       { file = "/var/keys/secret_file"; parentDirectory = { mode = "u=rwx,g=,o="; }; }
#     ];
#     users.talyz = {
#       directories = [
#         "Downloads"
#         "Music"
#         "Pictures"
#         "Documents"
#         "Videos"
#         "VirtualBox VMs"
#         { directory = ".gnupg"; mode = "0700"; }
#         { directory = ".ssh"; mode = "0700"; }
#         { directory = ".nixops"; mode = "0700"; }
#         { directory = ".local/share/keyrings"; mode = "0700"; }
#         ".local/share/direnv"
#       ];
#       files = [
#         ".screenrc"
#       ];
#     };
#   };
# }
