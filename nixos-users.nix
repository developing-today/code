let
  nixos-user-configuration =
    options:
    lib.attrsets.recursiveUpdate rec {
      inherit name;
      enable = true;
      uid = 1000;
      groups = [ "wheel" ];
      keys = [ (lib.user-key name) ];
      email = "nixos-user-${name}@developing-today.com";
      aliases = [
        "hi@developing-today.com"
        "abuse@developing-today.com"
        "dmca@developing-today.com"
        "drewrypope@gmail.com"
      ];
    } options;
in
lib.mapAttrs (
username: user-generator:
user-generator username
)
{
  user = nixos-user-configuration {};
}
# a nixos user contains a home manager user?
