{
  config,
  inputs,
  lib,
  pkgs,
  ...
}:
{
  imports = [
    (lib.from-root "hosts/nix/settings")
    inputs.determinate.nixosModules.default
  ];
  nix = {
    settings.flake-registry = ""; # https://github.com/NixOS/nix/issues/8953#issuecomment-1919310666
    # registry = lib.mkForce (lib.mapAttrs (_: value: { flake = value; }) inputs); # This will add each flake input as a registry. To make nix3 commands consistent with your flake
    # nixPath = lib.mapAttrsToList (key: value: "${key}=${value.to.path}") config.nix.registry; # This will additionally add your inputs to the system's legacy channels. Making legacy nix commands consistent as well, awesome!
    # package = pkgs.nixVersions.nix_2_23;
    optimise.automatic = true;
    gc = {
      automatic = true;
      dates = "weekly";
      options = "--delete-older-than 400d";
    };
  };
}
