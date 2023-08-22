{
  description = "Home Manager Flake";

  inputs = {
    # master then if it breaks unstable then if it breaks 23.11 or something.
    nixpkgs.url = "github:NixOS/nixpkgs"; # /nixos-unstable"; # /nixos-23.11";
    home-manager.url = "github:nix-community/home-manager";
  };

  outputs = {
    self,
    nixpkgs,
    home-manager,
    ...
  }: {
    homeManagerNixOsModules = stateVersion: [
      ({pkgs, ...}: {
        imports = [
          home-manager.nixosModules.home-manager
        ];
        home-manager.users.user = import ./users/user.nix {inherit stateVersion pkgs;};
      })
    ];
  };
}
