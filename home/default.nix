{
  # TODO: this doesn't work as-is
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
# this except home/ in this repo is shared-config and nixos/home/ is nixos hm config
# # shared-config.nix
# { config, pkgs, ... }: {
#   # Your shared configuration here
# }
# # NixOS configuration
# {
#   home-manager.users.username = import ./shared-config.nix;
# }
# # Standalone Home Manager configuration (home.nix)
# import ./shared-config.nix
