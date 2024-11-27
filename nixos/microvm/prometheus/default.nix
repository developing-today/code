{
  config,
  lib,
  pkgs,
  ...
}:
{
  microvm = {
    hypervisor = "qemu";
    # hypervisor = "cloud-hypervisor";
    vcpu = 2;
    mem = 1024;
    # console = "tty"; # or try "serial"
    # socket = "prometheus.sock";
    interfaces = [
      {
        type = "tap";
        id = "vm-prometheus";
        mac = "02:22:de:ad:be:ea";
      }
    ];

    shares = [
      {
        source = "/nix/store";
        mountPoint = "/nix/.ro-store";
        tag = "ro-store";
        proto = "virtiofs";
      }
    ];

    volumes = [
      {
        image = "prometheus-var.img";
        mountPoint = "/var";
        size = 8192;
      }
    ];
  };

  # Normal NixOS configuration past this point

  systemd.network.enable = true;

  systemd.network.networks."20-lan" = {
    matchConfig.Type = "ether";
    networkConfig = {
      DHCP = "yes";
      IPv6AcceptRA = true;
    };
  };

  networking = {
    hostName = "prometheus";
    firewall.package = pkgs.nftables;
    #firewall.allowedTCPPorts = [9090];
    nftables.enable = true;
  };

  services.prometheus = {
    enable = true;
  };

  system.stateVersion = "23.11";
}
