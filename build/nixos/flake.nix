{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-22.05";
    # XXX(Xe): this URL may change for you, such as github:Xe/gohello-http
    gohello.url = "git+https://tulpa.dev/cadey/gohello-http?ref=main";
  };

  outputs = { self, nixpkgs, gohello, ... }: {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix

        # add things here
        gohello.nixosModule
        ({ pkgs, ... }: {
          xeserv.services.gohello.enable = true;
        })
      ];
    };
    nixosConfigurations.DESKTOP-P1BL9NE = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix

        # add things here
        gohello.nixosModule
        ({ pkgs, ... }: {
          xeserv.services.gohello.enable = true;
        })
      ];
    };
  };
}
