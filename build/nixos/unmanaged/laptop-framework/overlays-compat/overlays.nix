final: prev:
with prev.lib;
let
  # Load the system config and get the `nixpkgs.overlays` option
  overlays = (import <nixpkgs/nixos> { }).config.nixpkgs.overlays;
  vscode = vscode-insider.overrideAttrs (oldAttrs: rec {
    src = (builtins.fetchTarball {
      url = "https://update.code.visualstudio.com/latest/linux-x64/insider";
      sha256 = "03nmmcr8canxnhxpsd2d5rfqi6d7njab4c3bpcqmfi9xbk3scx1a";
    });
    version = "latest";
  });
in
  # Apply all overlays to the input of the current "main" overlay
  foldl' (flip extends) (_: prev) overlays final
