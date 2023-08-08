{
  inputs.nixpkgs.url = "nixpkgs/nixos-unstable";
  inputs.zig-overlay.url = "mitchellh/zig-overlay";

  outputs = { self, nixpkgs, zig-overlay, ... }: {
    nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        { config, ... }: {
          nixpkgs.overlays = [ zig-overlay.defaultPackage ];
        }
      ];
    };
  };
}
