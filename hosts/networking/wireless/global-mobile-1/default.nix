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
  imports = [ (lib.from-root "hosts/networking/wireless") ];
  networking.wireless.networks."OnePlus 6".pskRaw = "ext:oneplus6";
  sops.secrets.wireless_global-mobile-1.sopsFile = lib.from-root "secrets/sops/common/networking/wireless/global-mobile-1.yaml";
}
