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
      # TEMPORARY (code#3906, found via code#2715): the nixpkgs bump
      # broke haskellPackages.regex-pcre (in the amd closure via
      # misfortune -> man-paths -> system-path) twice over. Between
      # the old and new unstable pins (6510408d -> 31ecbaca), the
      # Haskell builder stopped adding the pkg-config wrapper and
      # the pkgconfig-depends libraries to Haskell build
      # environments, and nixpkgs removed the EOL pcre C library
      # outright; regex-pcre's Cabal configure hard-requires both
      # the pkg-config program and libpcre.pc, so its build fails
      # deterministically. Restore the old build environment for
      # this one package: pkg-config in nativeBuildInputs, plus a
      # minimal vendored pcre 8.45 (the version the old pin shipped;
      # upstream's two patches are omitted -- one is PowerPC-only,
      # the other only matters to pcre's own test suite, which is
      # disabled here) in buildInputs. Remove once regex-pcre
      # builds stock again or misfortune leaves the closure; the
      # removal test is in code#3906.
      (
        final: prev:
        let
          pcre_8_45 = prev.stdenv.mkDerivation {
            pname = "pcre";
            version = "8.45";
            src = prev.fetchurl {
              url = "mirror://sourceforge/project/pcre/pcre/8.45/pcre-8.45.tar.bz2";
              sha256 = "sha256-Ta5v3NK7C7bDe1+Xwzwr6VTadDmFNpzdrDVG4yGL/7g=";
            };
            nativeBuildInputs = [ prev.updateAutotoolsGnuConfigScriptsHook ];
            configureFlags = [
              "--enable-unicode-properties"
              "--disable-cpp"
              "--enable-jit=auto"
            ];
            hardeningDisable = [ "shadowstack" ];
            doCheck = false;
          };
        in
        {
          haskellPackages = prev.haskellPackages.override {
            overrides = self: super: {
              regex-pcre = super.regex-pcre.overrideAttrs (old: {
                nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [ prev.pkg-config ];
                buildInputs = (old.buildInputs or [ ]) ++ [ pcre_8_45 ];
              });
            };
          };
        }
      )
      # Provides pkgs.openclaw + pkgs.openclawPackages for home/openclaw.
      inputs.nix-openclaw.overlays.default
      # inputs.waybar.overlays.default # ?? !! style.css
      # (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
    ];
  };
}
