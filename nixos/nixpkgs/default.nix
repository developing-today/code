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
      # TEMPORARY (code#2727, found via code#2715): patchelf 0.19.2 in
      # current nixpkgs corrupts the official Node.js 24.22.1 binary that
      # nix-openclaw repacks for OpenClaw -- the repacked `node` segfaults
      # on startup, its installCheck fails, and the failure cascades
      # openclaw -> home-manager -> toplevel. Proven 2026-10-30: the
      # pristine binary patched with patchelf 0.15.2 runs; the same binary
      # patched with 0.19.2 segfaults (gcc and glibc are identical across
      # the old/new pins and are not involved). The `patchelf` attribute
      # itself cannot be overridden (it is a nixpkgs bootstrap package;
      # overriding it breaks stdenv bootstrap evaluation), so the
      # autoPatchelfHook -- which resolves `patchelf` from PATH at build
      # time -- additionally propagates a patchelf 0.15.2 built from this
      # same package set. This overlay must precede the nix-openclaw
      # overlay below so that overlay's packages see the fixed hook.
      # Remove once the stock hook patches the Node.js repack cleanly
      # again; the removal test is in code#2727.
      (final: prev: {
        autoPatchelfHook = prev.autoPatchelfHook.overrideAttrs (old: {
          propagatedBuildInputs = (old.propagatedBuildInputs or [ ]) ++ [
            (prev.patchelf.overrideAttrs (oldPatchelf: {
              version = "0.15.2";
              src = prev.fetchurl {
                url = "https://github.com/NixOS/patchelf/releases/download/0.15.2/patchelf-0.15.2.tar.bz2";
                hash = "sha256-F3RfVkFZyOIo/EEtplogSLhGxLa0Igt3y/IkFuAvLXw=";
              };
            }))
          ];
        });
      })
      # Provides pkgs.openclaw + pkgs.openclawPackages for home/openclaw.
      inputs.nix-openclaw.overlays.default
      # inputs.waybar.overlays.default # ?? !! style.css
      # (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
    ];
  };
}
