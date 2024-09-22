let
  public-key = protocol: alias: builtins.readFile ./keys/${protocol}-${alias}.pub;
  group-key = alias: public-key "ssh-group" alias;
  host-key = alias: public-key "ssh-host" alias;
  user-key = alias: public-key "ssh-user" alias;
  nixos-user-configuration =
    name: options:
    lib.attrsets.recursiveUpdate rec {
      inherit name;
      enable = true;
      uid = 1000;
      groups = [ "wheel" ];
      keys = [ (user-key name) ];
      email = "nixos-user-${name}@developing-today.com";
      aliases = [
        "hi@developing-today.com"
        "abuse@developing-today.com"
        "dmca@developing-today.com"
        "drewrypope@gmail.com"
      ];
    } options;
in
{
  user = nixos-user-configuration "user";
}
# a nixos user contains a home manager user?
