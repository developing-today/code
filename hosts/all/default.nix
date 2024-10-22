{
  config,
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  ...
}:
{
  imports = with lib; [
    (from-root "hosts/boot")
    (from-root "hosts/i18n")
    (from-root "hosts/impermanence")
    (from-root "hosts/networking")
    (from-root "hosts/nix")
    (from-root "hosts/nixpkgs")
    (from-root "hosts/sops")
    (from-root "hosts/system")
    (from-root "hosts/tailscale-autoconnect")
    (from-root "hosts/time")
  ];
}
