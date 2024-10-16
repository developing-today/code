{ # TODO: this doesn't work as-is
  # homeConfigurations = mapAttrs (
  #   target: cfg:
  #   homeManagerConfiguration {
  #     pkgs = nixpkgs.legacyPackages.${cfg.system};
  #     extraSpecialArgs = {
  #       inherit inputs;
  #     };
  #     modules = [
  #       { home.stateVersion = cfg.stateVersion; }
  #       ./hm-modules/all.nix
  #       { inherit (cfg) my-nixos-hm; }
  #     ];
  #   }
  # ) (import ./hm-hosts.nix);
}
