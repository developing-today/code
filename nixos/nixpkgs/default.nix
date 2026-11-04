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
      # cutter 2.5.0 (code#5269): Qt 6.12's uic rejects the templated class
      # name "AddressableItemList<>" used in seven of cutter's .ui files
      # ("Invalid class name"), failing the build and cascading into the
      # amd toplevel. Mirror upstream's own fix: give the default
      # instantiation the plain name AddressableItemListView (a using-alias
      # added to AddressableItemList.h) and use it in the .ui files. Drop
      # this overlay once nixpkgs ships a cutter whose .ui files pass uic.
      (final: prev: {
        cutter = prev.cutter.overrideAttrs (old: {
          postPatch = (old.postPatch or "") + ''
            sed -i 's/AddressableItemList&lt;&gt;/AddressableItemListView/g' \
              src/dialogs/XrefsDialog.ui \
              src/widgets/BreakpointWidget.ui \
              src/widgets/FlagsWidget.ui \
              src/widgets/GlobalsWidget.ui \
              src/widgets/ListDockWidget.ui \
              src/widgets/SearchWidget.ui \
              src/widgets/StringsWidget.ui
            sed -i 's|^#endif // ADDRESSABLE_ITEM_LIST_H|using AddressableItemListView = AddressableItemList<CutterTreeView>;\n\n#endif // ADDRESSABLE_ITEM_LIST_H|' \
              src/widgets/AddressableItemList.h
          '';
        });
      })
      # Provides pkgs.openclaw + pkgs.openclawPackages for home/openclaw.
      inputs.nix-openclaw.overlays.default
      # inputs.waybar.overlays.default # ?? !! style.css
      # (final: prev: { omnix = inputs.omnix.packages.${system}.default; })
    ];
  };
}
