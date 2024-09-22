{ lib }:
let
  public-key = protocol: alias: builtins.readFile ./keys/${protocol}-${alias}.pub;
  group-key = alias: public-key "ssh-group" alias;
  host-key = alias: public-key "ssh-host" alias;
  user-key = alias: public-key "ssh-user" alias;
  # nixos-host-configuration = { options ? {} }:
  nixos-host-configuration =
    options: name:
    lib.attrsets.recursiveUpdate rec {
      inherit name;
      type = name;
      system = "x86_64-linux";
      stateVersion = "23.11";
      group-key = group-key name;
      email = "nixos-host-${name}@developing-today.com";
      sshKey = host-key name;
      hardware = ./hosts/common/modules/hardware-configuration/common;
    } options;
in
{
  nixos = nixos-host-configuration {
    hardware = ./hosts/common/modules/hardware-configuration/framework/13-inch/12th-gen-intel;
  };
  amd = nixos-host-configuration {
    hardware = ./hosts/common/modules/hardware-configuration/framework/13-inch/7040-amd;
  };
}
