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
  networking.wireless.networks."AmericInn".authProtocols = [ "NONE" ];
  sops.secrets.wireless_us-global-1.sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-global-1.yaml";
}
