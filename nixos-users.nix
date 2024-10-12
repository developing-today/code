inputs:
let
  lib = inputs.self.lib;
in
{
  user = lib.nixos-user-configuration { };
}
