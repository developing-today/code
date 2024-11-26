{ ... }:
{
  networking.interfaces.enp195s0f3u1 = {
    ipv4.addresses = [{
      address = "10.0.42.1";
      prefixLength = 24;
    }];
    # Make sure interface comes up automatically
    wakeOnLan.enable = true;
    useDHCP = false;
  };

  boot.kernel.sysctl."net.ipv4.ip_forward" = 1;

  networking.nat = {
    enable = true;
    externalInterface = "wlp1s0";
    internalInterfaces = [ "enp195s0f3u1" ];
  };
  services.dnsmasq = {
    enable = true;
    settings = {
      interface = "enp195s0f3u1";
      "dhcp-range" = "10.0.42.2,10.0.42.254,24h";
      "bind-dynamic" = true;  # Try this instead of bind-interfaces
    };
  };
  # services.dnsmasq = {
  #   enable = true;
  #   settings = {
  #     interface = "enp195s0f3u1";
  #     "dhcp-range" = "10.0.42.2,10.0.42.254,24h";
  #     "bind-interfaces" = true;
  #     "listen-address" = "10.0.42.1";
  #     "domain-needed" = true;
  #     "bogus-priv" = true;
  #   };
  # };

  # Ensure DHCP ports are open
  networking.firewall = {
    enable = true;
    allowedUDPPorts = [ 53 67 68 ];  # DNS, DHCP ports
    allowedTCPPorts = [ 53 ];     # DNS
  };
}
