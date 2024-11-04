{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  pkgs,
  ...
}:
{
  networking = {
    inherit hostName;
    useDHCP = true;
    enableIPv6 = true;
    firewall = {
      enable = true;
      allowedUDPPorts = [ config.services.tailscale.port ]; # needed? # put into tailscale-autoconnect?
    };
  };
}
# hostId = deadbeef # 8 unique hex chars
# domain
# useNetworkd = true;
# dhcpcd.persistent = true;
# nat
# https://search.nixos.org/options?channel=unstable&show=networking.supplicant&from=0&size=50&sort=relevance&type=packages&query=networking.supplicant
# https://nixos.wiki/wiki/Systemd-networkd
# systemd.network.netdevs
# https://discourse.nixos.org/t/imperative-declarative-wifi-networks-with-wpa-supplicant/12394/9
# allowedTCPPortRanges = [
#     { from = 4000; to = 4007; }
#     { from = 8000; to = 8010; }
# ];
# systemd.network.networks = let networkConfig = { DHCP = "yes"; DNSSEC = "yes"; DNSOverTLS = "yes"; DNS = [ "1.1.1.1" "1.0.0.1" ]; };
# boot.initrd.systemd.network.enable
# networking.useNetworkd
# systemd.networkd.enable
# It actually looks like there isn’t any options.systemd.networkd anyway (just options.systemd.network and boot.initrd.systemd.network), though systemd.network.networks.<name>.enable and systemd.network.netdevs.<name>.enable both refer to systemd.networkd; these docs definitely need attention.
# @efx: You probably just want to set systemd.network.enable = true and forget about boot.initrd.systemd.network entirely, unless you want to boot the device from another location on your network.
# systemd.services.systemd-udevd.restartIfChanged = false;
# systemd.services.tailscaled.after = ["NetworkManager-wait-online.service"]
# tailscale module??
# networking.useNetworkd = true;
# systemd.network.enable = true;
# systemd.network.wait-online.enable = false;
