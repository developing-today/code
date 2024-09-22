let
  public-key = protocol: alias: builtins.readFile ./keys/${protocol}-${alias}.pub;
  group-key = alias: public-key "ssh-group" alias;
  host-key = alias: public-key "ssh-host" alias;
  user-key = alias: public-key "ssh-user" alias;
  nixos-host-configuration = name: options ? {}:
    lib.attrsets.recursiveUpdate rec {
      inherit name;
      system = "x86_64-linux";
      stateVersion = "23.11";
      group-key = group-key name;
      email = "nixos-host-${name}@developing-today.com";
      sshKey = host-key name;
    } options;
in
{
  nixos = nixos-host-configuration "nixos";
  amd = nixos-host-configuration "amd";
}
