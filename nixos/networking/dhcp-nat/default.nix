{ lib, ... }:
let
  externalInterface = "wlp1s0";
  internalInterfaces = [
    "enp193s0f3u1"
    "enp195s0f3u1"
  ];
  interfaceIndices = builtins.listToAttrs (
    lib.lists.imap0 (idx: interface: {
      name = interface;
      value = idx;
    }) internalInterfaces
  );
  networkBase = index: {
    prefix = "10.0.${toString index}";
    gateway = "0";
    dhcpStart = "1";
    dhcpEnd = "254";
    prefixLength = 24;
    netmask = "255.255.255.0";
    lease = "24h";
  };
in
{
  systemd = {
    extraConfig = ''
      DefaultTimeoutStopSec=10s
    '';
    services = lib.merge [
      {
        # https://github.com/NixOS/nixpkgs/issues/154737
        # https://github.com/NixOS/nixpkgs/pull/155017/files
        systemd-networkd-wait-online.enable = false;
        NetworkManager-wait-online.enable = false;
      }
      (builtins.listToAttrs (
        lib.lists.imap0 (idx: interface: {
          name = "network-addresses-${interface}";
          value = {
            enable = false;
          };
        }) internalInterfaces
      ))
    ];
    network.wait-online = {
      timeout = 10;
      ignoredInterfaces = internalInterfaces;
      # extraArgs = "";
      enable = false;
      anyInterface = true;
    };
  };
  boot = {
    initrd.systemd.network.wait-online = {
      timeout = 10;
      ignoredInterfaces = internalInterfaces;
      # extraArgs
      enable = false;
      anyInterface = true;
    };
    kernel.sysctl."net.ipv4.ip_forward" = 1;
  };
  networking = {
    dhcpcd.wait = "background";
    # usePredictableInterfaceNames = false;
    usePredictableInterfaceNames = true;
    firewall = {
      enable = true;
      allowedUDPPorts = [
        53
        67
        68
      ];
      allowedTCPPorts = [ 53 ];
    };
    interfaces = builtins.foldl' (
      acc: interface:
      acc
      //
        (interface: {
          "${interface}" = {
            useDHCP = false;
            ipv4.addresses = [
              {
                address = "${(networkBase interfaceIndices.${interface}).prefix}.${
                  (networkBase interfaceIndices.${interface}).gateway
                }";
                prefixLength = (networkBase interfaceIndices.${interface}).prefixLength;
              }
            ];
            wakeOnLan.enable = true;
          };
        })
          interface
    ) { } internalInterfaces;
    nat = {
      enable = true;
      externalInterface = externalInterface;
      internalInterfaces = internalInterfaces;
    };
  };
  services = {
    dnsmasq = {
      enable = true;
      # alwaysKeepRunning = true;
      # package =
      resolveLocalQueries = false;
      settings = {
        interface = internalInterfaces;
        dhcp-range = builtins.genList (
          i:
          (
            index: interface:
            "${interface},${(networkBase index).prefix}.${(networkBase index).dhcpStart},${(networkBase index).prefix}.${(networkBase index).dhcpEnd},${(networkBase index).netmask},${(networkBase index).lease}"
          )
            i
            (builtins.elemAt internalInterfaces i)
        ) (builtins.length internalInterfaces);
        bind-dynamic = true;
      };
    };
    udev.extraRules = lib.concatMapStrings (interface: ''
      SUBSYSTEM=="net", ACTION=="add|move", NAME=="${interface}", TAG+="systemd", ENV{SYSTEMD_WANTS}="network-addresses-${interface}.service"
    '') internalInterfaces;
  };
}
