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
  imports = [
    (lib.from-root "hosts/boot")
    (lib.from-root "hosts/i18n")
    (lib.from-root "hosts/impermanence")
    (lib.from-root "hosts/networking")
    (lib.from-root "hosts/nix")
    (lib.from-root "hosts/nixpkgs")
    (lib.from-root "hosts/sops")
    (lib.from-root "hosts/system")
    (lib.from-root "hosts/tailscale-autoconnect")
    (lib.from-root "hosts/time")
  ];
}
