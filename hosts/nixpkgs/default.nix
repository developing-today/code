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
nixpkgs.pkgs = pkgs; # ??? Your system configures nixpkgs with an externally created instance. `nixpkgs.config` options should be passed when creating the instance instead.
# nixpkgs.overlays = pkgs.overlays;
# nixpkgs.config = {
#   allowBroken = true;
#   allowUnfree = true;
#   allowUnfreePredicate = _: true;
#   permittedInsecurePackages = [
#     "olm-3.2.16"
#     "electron" # le sigh
#     "qtwebkit-5.212.0-alpha4" # ???
#   ];
# }; # https://discourse.nixos.org/t/your-system-configures-nixpkgs-with-an-externally-created-instance/33802/2
}
