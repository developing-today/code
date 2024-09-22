let
  public-key = protocol: alias: builtins.readFile ./keys/${protocol}-${alias}.pub;
  group-key = alias: public-key "ssh-group" alias;
  host-key = alias: public-key "ssh-host" alias;
  user-key = alias: public-key "ssh-user" alias;
  default-home-manager-user-configuration = name: rec {
    system = "x86_64-linux";
    stateVersion = "23.11";
    home = rec {
      ide = rec {
        inherit name;
        enable = true;
        email = "home-manager-user-${name}@developing-today.com";
      };
      shell.enable = true;
      user = rec {
        inherit name;
        enable = true;
      };
    };
  };
  home-manager-user-configuration = name: options ? {}:
    lib.attrsets.recursiveUpdate (default-home-manager-user-configuration name) options
in
rec {
  "user@default" = home-manager-user-configuration "user";

  "user@nixos" = (parent: lib.attrsets.recursiveUpdate parent rec {
    home = rec {
      ide = rec {
        email = "nixos-home-manager-user-${parent.name}@developing-today.com";"
      };
    };
  }) ${user@default} ;
}
