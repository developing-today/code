{
  inputs.nixpkgs.url = "nixpkgs/nixos-unstable";
  inputs.zig-overlay.url = "github:mitchellh/zig-overlay";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs = { self, nixpkgs, zig-overlay, flake-utils, ... }: {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      system = "x85_64-linux";
      modules = [
        ./configuration.nix
        ({ pkgs, ... }: {
          nixpkgs.overlays = [
            (final: prev: { zigpkgs = zig-overlay.packages.${prev.system}; })
          ];
        })
      ];
    };
  };
}
