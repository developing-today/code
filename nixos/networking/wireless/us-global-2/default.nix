{ lib, ... }:
{
  imports = [ (lib.from-root "nixos/networking/wireless") ];
  networking.wireless.networks."IHG ONE REWARDS Free WI-FI".authProtocols = [ "NONE" ];
  # TODO: make a none secret file or something and use the 'key' attrib
  sops.secrets.wireless_us-global-2.sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-global-2.yaml";
}
