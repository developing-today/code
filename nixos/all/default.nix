{
  lib,
  ...
}:
{
  imports = with lib; [
    (from-root "nixos/boot")
    (from-root "nixos/i18n")
    (from-root "nixos/impermanence")
    (from-root "nixos/networking")
    (from-root "nixos/nix")
    (from-root "nixos/nixpkgs")
    (from-root "nixos/sops")
    (from-root "nixos/system")
    (from-root "nixos/tailscale-autoconnect")
    (from-root "nixos/time")
  ];
}
