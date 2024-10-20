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
  networking.wireless.networks."I win again, Lews Therin.".pskRaw = "ext:iwinagainlewstherin";
  sops.secrets."wireless_us-wi-1".sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-wi-1.yaml";
}
