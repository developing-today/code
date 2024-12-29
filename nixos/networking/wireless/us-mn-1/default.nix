{ lib, ... }:
{
  imports = [ (lib.from-root "nixos/networking/wireless") ];
  networking.wireless.networks."NETGEAR67".pskRaw = "ext:netgear67";
  sops.secrets.wireless_us-mn-1.sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-mn-1.yaml";
}
