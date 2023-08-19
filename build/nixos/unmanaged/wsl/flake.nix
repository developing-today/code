{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-22.05";
    # XXX(Xe): this URL may change for you, such as github:Xe/gohello-http
    # nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

    # home-manager.url = "github:nix-community/home-manager";
    # home-manager.inputs.nixpkgs.follows = "nixpkgs";

    gohello.url = "git+https://tulpa.dev/cadey/gohello-http?ref=main";
  };

  outputs = {
    self,
    nixpkgs,
    gohello,
    ...
  }: {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        gohello.nixosModule
        ({pkgs, ...}: {
          xeserv.services.gohello.enable = true;
        })
      ];
    };
    nixosConfigurations.DESKTOP-TOWER = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        gohello.nixosModule
        ({pkgs, ...}: {
          xeserv.services.gohello.enable = true;
        })
        home-manager.nixosModules.home-manager
        {
          home-manager.useGlobalPkgs = true;
          home-manager.useUserPackages = true;
          home-manager.users.nixos = import ./home.nix;

          # Optionally, use home-manager.extraSpecialArgs to pass
        }
      ];
    };
    nixosConfigurations.DESKTOP-P1BL9NE = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        gohello.nixosModule
        ({pkgs, ...}: {
          xeserv.services.gohello.enable = true;
        })
      ];
    };
  };
}
