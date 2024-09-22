rec {
  "user@default" = lib.home-manager-user-configuration "user";
  "user@nixos" = (parent: lib.attrsets.recursiveUpdate parent rec {
    home = rec {
      ide = rec {
        email = "nixos-home-manager-user-${parent.name}@developing-today.com";"
      };
    };
  }) ${"user@default"} ;
}
