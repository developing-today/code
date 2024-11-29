{
  lib,
  ...
}:
{
  imports = [ (lib.from-root "nixos/networking/wireless") ];
  networking.wireless.networks."OnePlus 6".pskRaw = "ext:oneplus6";
  sops.secrets.wireless_global-mobile-1.sopsFile = lib.from-root "secrets/sops/common/networking/wireless/global-mobile-1.yaml";
}
