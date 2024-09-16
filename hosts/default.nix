{
  inputs,
  outputs,
  lib,
  ...
}:
{ } // (import ./nixos) { inherit inputs outputs lib; }
