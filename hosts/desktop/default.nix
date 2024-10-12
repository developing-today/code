{
  inputs,
  hostName,
  host,
  system,
  stateVersion,
  lib,
  ...
}:
let
  pkgs = import inputs.nixpkgs {
    inherit system;
    config = {
      allowBroken = true;
      allowUnfree = true;
      allowUnfreePredicate = _: true;
      permittedInsecurePackages = [
        "olm-3.2.16"
        "electron"
        "qtwebkit-5.212.0-alpha4"
      ];
    };
    overlays = [
      inputs.vim.overlay.${system}
      inputs.yazi.overlays.default
      # inputs.waybar.overlays.default # ?? !! style.css
      # (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
    ];
  };
in
{
  imports = [
    (lib.from-root "hosts/tailscale-autoconnect")
    (lib.from-root "hosts/home")
    inputs.vim.nixosModules.${system}
    (lib.from-root "hosts/configuration.nix") # TODO: take this apart # this relies on magic overlays, ? todo: remove overlays from configuration.nix? then add inline let overlay configuration right here below this moduleArrayList.
    (lib.from-root "hosts/hyprland") # hyprland = would use flake for hyprland master but had annoying warning about waybar? todo try again. prefer flake. the config for this is setup in homeManager for reasons. could be brought out to nixos module would probably fit better due to my agonies
  ];
  system.stateVersion = stateVersion;
  nixpkgs.overlays = pkgs.overlays;
}
