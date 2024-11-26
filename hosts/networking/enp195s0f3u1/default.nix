{ ... }:
{
  boot.kernel.sysctl."net.ipv4.ip_forward" = 1;
  networking = {
    firewall = {
      enable = true;
      allowedUDPPorts = [ 53 67 68 ];
      allowedTCPPorts = [ 53 ];
    };
    interfaces.enp195s0f3u1 = {
      ipv4.addresses = [{
        address = "10.0.42.1";
        prefixLength = 24;
      }];
      wakeOnLan.enable = true;
      useDHCP = false;
    };
    nat = {
      enable = true;
      externalInterface = "wlp1s0";
      internalInterfaces = [ "enp195s0f3u1" ];
    };
  };
  services.dnsmasq = {
    enable = true;
    settings = {
      interface = [ "enp195s0f3u1" "enp193s0f3u1" ];
      "dhcp-range" = [
        "enp195s0f3u1,10.0.42.2,10.0.42.254,24h"
        "enp193s0f3u1,10.0.42.2,10.0.42.254,24h"
      ];
      "bind-dynamic" = true;
    };
  };
}
