{
  description = "Home Manager Flake";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.home-manager.url = "github:nix-community/home-manager";

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
