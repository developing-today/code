inputs:
let
  inherit (inputs.self) lib;
in
{
  user = lib.nixos-user-configuration { };
}
