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
    # hostId = deadbeef # 8 unique hex chars
    # domain
    useDHCP = true;
    # useNetworkd = true;
    # dhcpcd.persistent = true;
    enableIPv6 = true;
    # nat
    # https://search.nixos.org/options?channel=unstable&show=networking.supplicant&from=0&size=50&sort=relevance&type=packages&query=networking.supplicant
    # https://nixos.wiki/wiki/Systemd-networkd
    # systemd.network.netdevs
    # https://discourse.nixos.org/t/imperative-declarative-wifi-networks-with-wpa-supplicant/12394/9
    firewall = {
      enable = true;
      allowedUDPPorts = [ config.services.tailscale.port ]; # needed?
      # allowedTCPPortRanges = [
      #     { from = 4000; to = 4007; }
      #     { from = 8000; to = 8010; }
      # ];
    };
    networkmanager = {
      enable = false;
      unmanaged = [
        "*"
        "except:type:wwan"
        "except:type:gsm"
      ];
    };
    wireless = {
      enable = true;
      # userControlled.enable = true;
      scanOnLowSignal = true;
      fallbackToWPA2 = true;
      secretsFile = config.sops.secrets.wireless.path;
      networks = import (lib.from-root "hosts/networking/wireless/us-wi-1");
      allowAuxiliaryImperativeNetworks = true; # TODO: can we disable this?
      userControlled = {
        enable = true;
        group = "network";
      };
      # whats extraConfig.update_config=1 do?
      extraConfig = ''
        update_config=1
      '';
    };
  };
  sops.secrets."wireless" = {
    # TODO: us-wi-1 module in hosts/networking/wireless/us-wi-1, make-wireless if wireless is not []
    sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-wi-1.yaml";
  };
  # # Ensure group exists
  # this would be for users that aren't root or sudoers or doassers or whatever
  users.groups.network = { };
  # TODO: check if not needed?? https://github.com/NixOS/nixpkgs/pull/305649
  # systemd.services.wpa_supplicant.preStart = "touch /etc/wpa_supplicant.conf";
}
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
