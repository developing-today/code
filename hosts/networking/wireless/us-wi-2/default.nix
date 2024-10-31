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
  imports = [
    (lib.from-root "hosts/networking/wireless")
  ];
  networking.wireless.networks."TDS417".pskRaw = "ext:TDS417";
  sops.secrets.wireless_us-wi-2.sopsFile = lib.from-root "secrets/sops/common/networking/wireless/us-wi-2.yaml";
}
