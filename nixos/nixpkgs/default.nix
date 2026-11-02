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
      # TEMPORARY (code#4167, found via code#2715): awscli 1.44.21
      # fails pythonRuntimeDepsCheck on the current pins -- it pins
      # botocore==1.42.31 and s3transfer>=0.16.0,<0.17.0, but every
      # pin now ships botocore 1.43.79 and s3transfer 0.19.2 (the tree
      # is mid-transition). awscli's own expression notes botocore
      # must not be relaxed (the two are developed in lockstep), so
      # give awscli dedicated botocore/s3transfer derivations rolled
      # back to the versions it pins; the set's botocore/s3transfer
      # stay current for all other consumers. Remove once stock
      # awscli passes the check again; tracked in code#4167.
      (
        final: prev:
        let
          botocore_1_42_31 = prev.python3Packages.botocore.overridePythonAttrs (old: {
            version = "1.42.31";
            src = prev.fetchFromGitHub {
              owner = "boto";
              repo = "botocore";
              tag = "1.42.31";
              hash = "sha256-avuv1uXKMeSr3SL+BI9XW8tDCQM/dlXFn590di3S03k=";
            };
          });
          s3transfer_0_16_0 = prev.python3Packages.s3transfer.overridePythonAttrs (old: {
            # s3transfer itself depends on the set's botocore; point it
            # at the pinned one too, or awscli's closure contains both
            # botocores and pythonCatchConflictsPhase fails the build.
            dependencies = map (d: if (d.pname or "") == "botocore" then botocore_1_42_31 else d) (
              old.dependencies or [ ]
            );
            version = "0.16.0";
            src = prev.fetchFromGitHub {
              owner = "boto";
              repo = "s3transfer";
              tag = "0.16.0";
              hash = "sha256-dpDlsZtLjd6r4kLkIDPG6ZPFFs6/4elYiHk2HDpa9+4=";
            };
          });
        in
        {
          awscli = prev.awscli.overridePythonAttrs (old: {
            # The expression's installCheck greps `aws --version` for
            # the SET's botocore version; with the pairing overridden
            # it must grep for the pinned one instead (same assertion,
            # pairing actually in force).
            installCheckPhase = ''
              runHook preInstallCheck

              $out/bin/aws --version | grep "1.42.31"
              $out/bin/aws --version | grep "1.44.21"

              runHook postInstallCheck
            '';
            dependencies = map (
              d:
              if (d.pname or "") == "botocore" then
                botocore_1_42_31
              else if (d.pname or "") == "s3transfer" then
                s3transfer_0_16_0
              else
                d
            ) old.dependencies;
          });
        }
      )
      # TEMPORARY (code#4277, found via code#2715): cargo-llvm-cov's
      # test suite fails deterministically on the current pins --
      # rustc 1.99 writes raw profile format version 11, but the
      # expression pins llvmPackages_22, whose llvm-profdata expects
      # version 10 ("raw profile version mismatch ... error: no
      # profile can be merged", 14 of 20 integration tests fail; the
      # expression's header comment names this exact failure mode).
      # Build it against llvmPackages_23, whose profdata reads v11;
      # the full suite passes. Remove once the expression's pinned
      # llvm matches rustc again; tracked in code#4277.
      (final: prev: {
        cargo-llvm-cov = prev.cargo-llvm-cov.override {
          llvmPackages_22 = prev.llvmPackages_23;
        };
      })
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
