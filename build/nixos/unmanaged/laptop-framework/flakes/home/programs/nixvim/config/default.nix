{ enableModules, pkgs, ... }: {
  imports = let
    moduleFiles = builtins.filter (n: builtins.match ".*\\.nix" n != null)
      (builtins.attrNames (builtins.readDir ./modules));
  in map (n: enableModules (import (./modules + "/${n}") { inherit pkgs; }))
  moduleFiles;
}
