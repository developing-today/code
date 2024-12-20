inputs:
let
  lib = inputs.self.lib;
in
{
  "user@default" = lib.home-manager-user-configuration "user";
  "user@nixos" =
    (
      parent:
      lib.attrsets.recursiveUpdate parent {
        home = {
          ide = {
            email = "nixos-home-manager-user-${parent.name}@developing-today.com";
          };
        };
      }
    )
      "user@default";
}
