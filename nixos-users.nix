lib.mapAttrs (username: user-generator: user-generator username) rec {
  user = lib.nixos-user-configuration { };
}
