{ lib, ... }:
let
  externalInterface = "wlp1s0";
  internalInterfaces = [
    "enp193s0f3u1"
    "enp195s0f3u1"
  ];
  # Create mapping of interface to index
  interfaceIndices = builtins.listToAttrs (
    lib.lists.imap0 (idx: interface: { name = interface; value = idx; }) internalInterfaces
  );
  networkBase = index: {
    prefix = "10.0.${toString index}";
    gateway = "1";
    dhcpStart = "2";
    dhcpEnd = "254";
    prefixLength = 24;
    netmask = "255.255.255.0";
  };
  makeIpConfig = interface: {
    "${interface}" = {
      ipv4.addresses = [
        {
          address = "${(networkBase interfaceIndices.${interface}).prefix}.${(networkBase interfaceIndices.${interface}).gateway}";
          prefixLength = (networkBase interfaceIndices.${interface}).prefixLength;
        }
      ];
      wakeOnLan.enable = true;
      useDHCP = false;
    };
  };
  makeDhcpRange = index: interface:
    "${interface},${(networkBase index).prefix}.${(networkBase index).dhcpStart},${(networkBase index).prefix}.${(networkBase index).dhcpEnd},${(networkBase index).netmask},24h";
in
{
  boot.kernel.sysctl."net.ipv4.ip_forward" = 1;
  networking = {
    firewall = {
      enable = true;
      allowedUDPPorts = [
        53
        67
        68
      ];
      allowedTCPPorts = [ 53 ];
    };
    interfaces = builtins.foldl' (acc: interface: acc // makeIpConfig interface) { } internalInterfaces;
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
      dhcp-range = builtins.genList (i: makeDhcpRange i (builtins.elemAt internalInterfaces i)) (
        builtins.length internalInterfaces
      );
      bind-dynamic = true;
    };
  };
}
