{ inputs, system, ... }:
{
  nixpkgs.pkgs = import inputs.nixpkgs {
    inherit system;
    config = {
      allowBroken = true;
      allowUnfree = true;
      allowUnfreePredicate = _: true;
      # Android SDK (pkgs.androidenv) is unfree and gates its derivation on
      # license acceptance; without this, evaluating the SDK throws. Set here
      # rather than via nixpkgs.config because this module assigns nixpkgs.pkgs
      # directly, which makes nixpkgs.config a no-op.
      android_sdk.accept_license = true;
      permittedInsecurePackages = [
        "olm-3.2.16"
        "electron"
        "qtwebkit-5.212.0-alpha4"
      ];
    };
    overlays = [
      inputs.self.vimOverlay.${system}
      inputs.yazi.overlays.default
      # Provides pkgs.openclaw + pkgs.openclawPackages for home/openclaw.
      inputs.nix-openclaw.overlays.default
      # inputs.waybar.overlays.default # ?? !! style.css
      # (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
    ];
  };
}
