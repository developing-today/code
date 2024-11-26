{ ... }:
let
  externalInterface = "wlp1s0";
  internalInterfaces = [ "enp195s0f3u1" "enp193s0f3u1" ];
  makeIpConfig = interface: {
    "${interface}" = {
      ipv4.addresses = [{
        address = "10.0.42.1";
        prefixLength = 24;
      }];
      wakeOnLan.enable = true;
      useDHCP = false;
    };
  };
  makeDhcpRange = interface: "${interface},10.0.42.2,10.0.42.254,24h";
in
{
 boot.kernel.sysctl."net.ipv4.ip_forward" = 1;

 networking = {
   firewall = {
     enable = true;
     allowedUDPPorts = [ 53 67 68 ];
     allowedTCPPorts = [ 53 ];
   };
   interfaces = builtins.foldl' (acc: interface: acc // makeIpConfig interface) {} internalInterfaces;
   nat = {
     enable = true;
     externalInterface = externalInterface;
     internalInterfaces = internalInterfaces;
   };
 };

 services.dnsmasq = {
   enable = true;
   settings = {
     interface = internalInterfaces;
     dhcp-range = map makeDhcpRange internalInterfaces;
     bind-dynamic = true;
   };
 };
}
